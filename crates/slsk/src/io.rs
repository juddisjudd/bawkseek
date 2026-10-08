use std::io;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

/// Large enough for the browse reply of a very big share, small enough that a hostile peer cannot exhaust memory.
pub(crate) const MAX_FRAME: usize = 64 * 1024 * 1024;

pub(crate) async fn read_frame(stream: &mut (impl AsyncRead + Unpin)) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len).await?;
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let mut frame = vec![0; len];
    stream.read_exact(&mut frame).await?;
    Ok(frame)
}

pub(crate) fn split_u32(frame: &[u8]) -> Option<(u32, &[u8])> {
    let code = u32::from_le_bytes(frame.get(..4)?.try_into().ok()?);
    Some((code, &frame[4..]))
}

pub(crate) fn split_u8(frame: &[u8]) -> Option<(u8, &[u8])> {
    Some((*frame.first()?, &frame[1..]))
}

/// Writes queued frames in order; dropping every sender closes the connection.
pub(crate) fn spawn_writer(mut half: OwnedWriteHalf) -> UnboundedSender<Vec<u8>> {
    let (tx, mut rx) = unbounded_channel::<Vec<u8>>();
    tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            if half.write_all(&frame).await.is_err() {
                break;
            }
        }
        let _ = half.shutdown().await;
    });
    tx
}

pub(crate) async fn connect(addr: &str, timeout: Duration) -> io::Result<TcpStream> {
    let stream = tokio::time::timeout(timeout, TcpStream::connect(addr))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "connect timed out"))??;
    stream.set_nodelay(true)?;
    let keepalive = socket2::TcpKeepalive::new()
        .with_time(Duration::from_secs(60))
        .with_interval(Duration::from_secs(10));
    socket2::SockRef::from(&stream).set_tcp_keepalive(&keepalive)?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reads_frames_and_refuses_huge_ones() {
        let mut data: &[u8] = &[3, 0, 0, 0, 1, 2, 3, 0xff, 0xff, 0xff, 0xff];
        assert_eq!(read_frame(&mut data).await.unwrap(), [1, 2, 3]);
        assert_eq!(
            read_frame(&mut data).await.unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn splits_codes() {
        assert_eq!(split_u32(&[1, 0, 0, 0, 9]), Some((1, &[9][..])));
        assert_eq!(split_u8(&[4, 9]), Some((4, &[9][..])));
        assert_eq!(split_u32(&[1]), None);
    }
}
