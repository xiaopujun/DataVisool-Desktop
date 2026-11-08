/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Per-operation socket deadlines, independent of the whole-query deadline.

use std::{
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    time::{sleep, Sleep},
};

pub(super) struct SocketTimeoutStream<S> {
    inner: S,
    timeout: Option<Duration>,
    read_timer: Option<Pin<Box<Sleep>>>,
    write_timer: Option<Pin<Box<Sleep>>>,
}

impl<S> SocketTimeoutStream<S> {
    pub(super) fn new(inner: S, timeout: Option<Duration>) -> Self {
        Self {
            inner,
            timeout,
            read_timer: None,
            write_timer: None,
        }
    }
}

fn poll_deadline(
    timeout: Option<Duration>,
    timer: &mut Option<Pin<Box<Sleep>>>,
    cx: &mut Context<'_>,
    operation: &str,
) -> Poll<io::Result<()>> {
    let Some(timeout) = timeout else {
        return Poll::Pending;
    };
    let timer = timer.get_or_insert_with(|| Box::pin(sleep(timeout)));
    match timer.as_mut().poll(cx) {
        Poll::Ready(()) => Poll::Ready(Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("SQL Server socket {operation} timed out"),
        ))),
        Poll::Pending => Poll::Pending,
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for SocketTimeoutStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_read(cx, buf) {
            Poll::Ready(result) => {
                this.read_timer = None;
                Poll::Ready(result)
            }
            Poll::Pending => poll_deadline(this.timeout, &mut this.read_timer, cx, "read"),
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for SocketTimeoutStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_write(cx, buf) {
            Poll::Ready(result) => {
                this.write_timer = None;
                Poll::Ready(result)
            }
            Poll::Pending => {
                match poll_deadline(this.timeout, &mut this.write_timer, cx, "write") {
                    Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
                    _ => Poll::Pending,
                }
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_flush(cx) {
            Poll::Ready(result) => {
                this.write_timer = None;
                Poll::Ready(result)
            }
            Poll::Pending => poll_deadline(this.timeout, &mut this.write_timer, cx, "flush"),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_shutdown(cx) {
            Poll::Ready(result) => {
                this.write_timer = None;
                Poll::Ready(result)
            }
            Poll::Pending => poll_deadline(this.timeout, &mut this.write_timer, cx, "shutdown"),
        }
    }
}

#[cfg(test)]
#[path = "sql_server_io_tests.rs"]
mod tests;
