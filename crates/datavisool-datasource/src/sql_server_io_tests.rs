/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use super::*;
use std::future::poll_fn;
use tokio::{
    net::{TcpListener, TcpStream},
    time::{timeout, Instant},
};

const IDLE: Duration = Duration::from_millis(30);

async fn read_byte<S: AsyncRead + Unpin>(stream: &mut S) -> io::Result<()> {
    let mut byte = [0];
    poll_fn(|cx| Pin::new(&mut *stream).poll_read(cx, &mut ReadBuf::new(&mut byte))).await
}

#[tokio::test]
async fn stalled_socket_read_times_out_before_outer_login_or_query_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let socket = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (_peer, _) = listener.accept().await.unwrap();
    let mut stream = SocketTimeoutStream::new(socket, Some(IDLE));
    let error = timeout(Duration::from_secs(1), read_byte(&mut stream))
        .await
        .expect("the socket deadline should fire before the outer deadline")
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(error.to_string().contains("socket read"));
}

struct PendingIo;

impl AsyncRead for PendingIo {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Poll::Pending
    }
}

impl AsyncWrite for PendingIo {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Pending
    }
    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Pending
    }
    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Pending
    }
}

#[tokio::test]
async fn stalled_socket_writes_flushes_and_shutdowns_are_bounded() {
    let mut stream = SocketTimeoutStream::new(PendingIo, Some(IDLE));
    let error = poll_fn(|cx| Pin::new(&mut stream).poll_write(cx, &[1]))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    let mut stream = SocketTimeoutStream::new(PendingIo, Some(IDLE));
    let error = poll_fn(|cx| Pin::new(&mut stream).poll_flush(cx))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    let mut stream = SocketTimeoutStream::new(PendingIo, Some(IDLE));
    let error = poll_fn(|cx| Pin::new(&mut stream).poll_shutdown(cx))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
}

#[tokio::test]
async fn zero_socket_timeout_remains_pending_until_the_outer_deadline() {
    let mut stream = SocketTimeoutStream::new(PendingIo, None);
    assert!(timeout(IDLE, read_byte(&mut stream)).await.is_err());
}

struct DelayedBytes {
    pending: Option<Pin<Box<Sleep>>>,
}

impl AsyncRead for DelayedBytes {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let timer = self
            .pending
            .get_or_insert_with(|| Box::pin(sleep(IDLE / 2)));
        match timer.as_mut().poll(cx) {
            Poll::Ready(()) => {
                self.pending = None;
                buf.put_slice(&[1]);
                Poll::Ready(Ok(()))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

#[tokio::test]
async fn successful_reads_reset_the_idle_deadline() {
    let mut stream = SocketTimeoutStream::new(DelayedBytes { pending: None }, Some(IDLE));
    let start = Instant::now();
    for _ in 0..4 {
        read_byte(&mut stream).await.unwrap();
    }
    // The stream can communicate longer than one socket timeout as long as
    // every individual read makes progress before its deadline.
    assert!(start.elapsed() > IDLE);
}
