// SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tracing::{debug, info};

pub struct HealthServer {
    ready: Arc<AtomicBool>,
}

impl HealthServer {
    pub fn new(ready: Arc<AtomicBool>) -> Self {
        Self { ready }
    }

    pub fn set_ready(&self) {
        self.ready.store(true, Ordering::Release);
    }

    pub async fn serve(self, port: u16) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        let listener = TcpListener::bind(addr).await?;
        info!(port, "Health check endpoint listening");

        let ready = self.ready;
        loop {
            let (stream, _) = listener.accept().await?;
            let io = TokioIo::new(stream);
            let ready = ready.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req| {
                    let ready = ready.clone();
                    async move { Ok::<_, Infallible>(handle_request(req, &ready)) }
                });
                if let Err(err) = hyper_util::server::conn::auto::Builder::new(
                    hyper_util::rt::TokioExecutor::new(),
                )
                .serve_connection(io, svc)
                .await
                {
                    debug!(error = %err, "Health check connection error");
                }
            });
        }
    }
}

fn handle_request(
    req: Request<hyper::body::Incoming>,
    ready: &AtomicBool,
) -> Response<Full<Bytes>> {
    match req.uri().path() {
        "/readyz" => {
            if ready.load(Ordering::Acquire) {
                Response::builder()
                    .status(StatusCode::OK)
                    .body(Full::new(Bytes::from("ok")))
                    .unwrap()
            } else {
                Response::builder()
                    .status(StatusCode::SERVICE_UNAVAILABLE)
                    .body(Full::new(Bytes::from("not ready")))
                    .unwrap()
            }
        }
        _ => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Full::new(Bytes::from("not found")))
            .unwrap(),
    }
}
