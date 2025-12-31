// RASPBERRY PI camera mjpeg streamer
// Copyright (c) 2025 [David Ron - rd789x@gmail.com]

use axum::{
    body::Body,
    extract::State,
    http::{
        header::{self, HeaderName},
        HeaderValue,
    },
    response::{Response, Html},
    routing::get,
    serve::ListenerExt,
    Router,
};
use bytes::{BufMut, Bytes, BytesMut};
use futures::Stream;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::{AppSink, AppSinkCallbacks};
use socket2::{SockRef, TcpKeepalive};
use std::{collections::VecDeque, sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::watch};

#[derive(Clone)]
struct AppState {
    // Latest-frame semantics: each client subscribes and always reads the newest JPEG.
    frame_tx: watch::Sender<Bytes>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    gst::init()?;

    // Latest-frame channel (constant memory, no backlog).
    let (frame_tx, _frame_rx) = watch::channel(Bytes::new());
    let state = Arc::new(AppState { frame_tx });

    // Spawn GStreamer pipeline thread
    {
        let frame_tx = state.frame_tx.clone();
        std::thread::spawn(move || {
            // For tunnels: tune fps/quality to stay below uplink capacity.
            let pipeline_str = "\
                libcamerasrc ! \
                video/x-raw,format=NV12,width=1280,height=960,framerate=25/1 ! \
                jpegenc quality=70 ! \
                appsink name=sink emit-signals=true max-buffers=1 drop=true";

            let pipeline = match gst::parse::launch(pipeline_str) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("GStreamer parse/launch error: {e}");
                    return;
                }
            };

            let pipeline = match pipeline.dynamic_cast::<gst::Pipeline>() {
                Ok(p) => p,
                Err(_) => {
                    eprintln!("GStreamer error: pipeline is not a gst::Pipeline");
                    return;
                }
            };

            let appsink = match pipeline.by_name("sink").and_then(|e| e.dynamic_cast::<AppSink>().ok())
            {
                Some(s) => s,
                None => {
                    eprintln!("GStreamer error: appsink named 'sink' not found or wrong type");
                    return;
                }
            };

            // Configure appsink caps
            appsink.set_caps(Some(&gst::Caps::builder("image/jpeg").build()));

            appsink.set_callbacks(
                AppSinkCallbacks::builder()
                    .new_sample(move |sink| {
                        let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;

                        if let Some(buffer) = sample.buffer() {
                            if let Ok(map) = buffer.map_readable() {
                                // One copy here (from GStreamer buffer into owned Bytes),
                                // then shared cheaply to all clients.
                                let jpeg = Bytes::copy_from_slice(map.as_slice());
                                frame_tx.send_replace(jpeg);
                            }
                        }

                        Ok(gst::FlowSuccess::Ok)
                    })
                    .build(),
            );

            if let Err(e) = pipeline.set_state(gst::State::Playing) {
                eprintln!("GStreamer error: failed to set Playing state: {e:?}");
                return;
            }

            // Keep the thread alive; report errors if any.
            if let Some(bus) = pipeline.bus() {
                for msg in bus.iter_timed(gst::ClockTime::NONE) {
                    match msg.view() {
                        gst::MessageView::Error(err) => {
                            eprintln!("GStreamer error: {}", err.error());
                            break;
                        }
                        gst::MessageView::Eos(_) => break,
                        _ => {}
                    }
                }
            }
        });
    }

    // Axum app
	let app = Router::new()
		.route("/", get(index))
		.route("/stream", get(mjpeg_handler))
		.with_state(state);


    println!("Listening at http://0.0.0.0:8080/stream");

    let listener = TcpListener::bind("0.0.0.0:8080")
        .await?
		.tap_io(|tcp_stream: &mut tokio::net::TcpStream| {
			// Disable Nagle
			if let Err(err) = tcp_stream.set_nodelay(true) {
				eprintln!("failed to set TCP_NODELAY: {err}");
			}

			// Keepalive via socket2 on the underlying std socket
			let sock = SockRef::from(tcp_stream.as_ref());

			if let Err(err) = sock.set_keepalive(true) {
				eprintln!("failed to enable SO_KEEPALIVE: {err}");
				return;
			}

			let ka = TcpKeepalive::new()
				.with_time(Duration::from_secs(60))
				.with_interval(Duration::from_secs(10));

			if let Err(err) = sock.set_tcp_keepalive(&ka) {
				eprintln!("failed to set TCP keepalive params: {err}");
			}
		});

    axum::serve(listener, app).await?;
    Ok(())
}

async fn index() -> Html<&'static str> {
    Html(include_str!("./index.html"))
}

async fn mjpeg_handler(State(state): State<Arc<AppState>>) -> Response {
    let rx = state.frame_tx.subscribe();

    // 2-chunk-per-frame MJPEG stream: (header-with-optional-leading-CRLF) + jpeg
    let body_stream = watch_to_multipart_stream(rx);

    let body = Body::from_stream(body_stream);

    let mut response = Response::new(body);
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("multipart/x-mixed-replace; boundary=FRAME"),
    );

    // Discourage buffering/caching in browsers and intermediary proxies.
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-store, must-revalidate"),
    );
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
        .headers_mut()
        .insert(header::EXPIRES, HeaderValue::from_static("0"));

    // Some reverse proxies honor this; harmless otherwise.
    const X_ACCEL_BUFFERING: HeaderName = HeaderName::from_static("x-accel-buffering");
    response.headers_mut().insert(X_ACCEL_BUFFERING, HeaderValue::from_static("no"));

    response
}

// Convert watch receiver into a multipart MJPEG stream of Bytes.
// Semantics: always send the latest frame (no app-level queue).
// Coalescing: 2 chunks per frame by folding the previous trailing CRLF into the next header.
fn watch_to_multipart_stream(
    rx: watch::Receiver<Bytes>,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static {
    futures::stream::unfold(
        (rx, VecDeque::<Bytes>::new(), true /* first */),
        |(mut rx, mut pending, mut first)| async move {
            // Emit any prepared chunks first.
            if let Some(next) = pending.pop_front() {
                return Some((Ok(next), (rx, pending, first)));
            }

            // Wait for the next frame update.
            if rx.changed().await.is_err() {
                return None; // sender dropped => end stream
            }

            let jpeg = rx.borrow().clone();
            if jpeg.is_empty() {
                // Initial empty value - wait for a real frame.
                return Some((Ok(Bytes::new()), (rx, pending, first)));
            }

            // Header chunk: prepend "\r\n" for all frames after the first (saves a chunk).
            // Capacity: typical header is well under 128 bytes, but Content-Length digits vary.
            let mut header = BytesMut::with_capacity(160);

            if first {
                header.put_slice(b"--FRAME\r\n");
                first = false;
            } else {
                header.put_slice(b"\r\n--FRAME\r\n");
            }

            header.put_slice(b"Content-Type: image/jpeg\r\nContent-Length: ");
            put_usize_ascii(&mut header, jpeg.len());
            header.put_slice(b"\r\n\r\n");

            // Two chunks: header, then JPEG bytes (no copy of JPEG here).
            pending.push_back(header.freeze());
            pending.push_back(jpeg);

            let out = pending.pop_front().unwrap();
            Some((Ok(out), (rx, pending, first)))
        },
    )
}

// Write usize as ASCII digits into a BytesMut without fmt allocation.
fn put_usize_ascii(dst: &mut BytesMut, mut n: usize) {
    let mut buf = [0u8; 20]; // enough for 64-bit usize
    let mut i = buf.len();

    if n == 0 {
        i -= 1;
        buf[i] = b'0';
    } else {
        while n > 0 {
            i -= 1;
            buf[i] = b'0' + (n % 10) as u8;
            n /= 10;
        }
    }

    dst.put_slice(&buf[i..]);
}
