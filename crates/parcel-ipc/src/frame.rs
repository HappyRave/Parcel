use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Largest accepted frame body; anything bigger is a protocol error, not a real message.
pub const MAX_FRAME: u32 = 16 << 20;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("bad message: {0}")]
    Json(#[from] serde_json::Error),
    #[error("frame of {0} bytes is over the limit")]
    TooLarge(usize),
    #[error("protocol {theirs} doesn't match ours ({ours})")]
    Protocol { ours: u32, theirs: u32 },
    #[error("unexpected message: {0}")]
    Unexpected(String),
}

/// Writes `msg` as a little-endian `u32` length followed by its JSON.
///
/// # Errors
/// On I/O failure or a message over [`MAX_FRAME`].
pub async fn write<W, T>(w: &mut W, msg: &T) -> Result<(), Error>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let body = serde_json::to_vec(msg)?;
    let len = u32::try_from(body.len())
        .ok()
        .filter(|&n| n <= MAX_FRAME)
        .ok_or(Error::TooLarge(body.len()))?;
    let mut buf = Vec::with_capacity(4 + body.len());
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(&body);
    w.write_all(&buf).await?;
    w.flush().await?;
    Ok(())
}

/// Reads one frame. `Ok(None)` means the other side closed cleanly between frames.
///
/// # Errors
/// On I/O failure, a truncated frame, an oversized length or invalid JSON.
pub async fn read<R, T>(r: &mut R) -> Result<Option<T>, Error>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut len = [0u8; 4];
    match r.read_exact(&mut len).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_le_bytes(len);
    if len > MAX_FRAME {
        return Err(Error::TooLarge(len as usize));
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body).await?;
    Ok(Some(serde_json::from_slice(&body)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{AppRate, HostRate, ServerMsg, Tick};

    #[tokio::test]
    async fn frames_round_trip() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        let tick = ServerMsg::Tick(Tick {
            at: 1,
            down: 2,
            up: 3,
            apps: Vec::new(),
        });
        write(&mut a, &tick).await.unwrap();
        write(&mut a, &tick).await.unwrap();
        drop(a);
        assert_eq!(
            read::<_, ServerMsg>(&mut b).await.unwrap(),
            Some(tick.clone())
        );
        assert_eq!(read::<_, ServerMsg>(&mut b).await.unwrap(), Some(tick));
        assert_eq!(read::<_, ServerMsg>(&mut b).await.unwrap(), None);
    }

    #[tokio::test]
    async fn oversized_length_is_refused() {
        let (mut a, mut b) = tokio::io::duplex(64);
        a.write_all(&(MAX_FRAME + 1).to_le_bytes()).await.unwrap();
        assert!(matches!(
            read::<_, ServerMsg>(&mut b).await,
            Err(Error::TooLarge(_))
        ));
    }

    #[tokio::test]
    async fn truncated_frame_is_an_error() {
        let (mut a, mut b) = tokio::io::duplex(64);
        a.write_all(&10u32.to_le_bytes()).await.unwrap();
        a.write_all(b"{}").await.unwrap();
        drop(a);
        assert!(matches!(
            read::<_, ServerMsg>(&mut b).await,
            Err(Error::Io(_))
        ));
    }

    #[test]
    fn tick_wire_shape() {
        let json = serde_json::to_string(&ServerMsg::Tick(Tick {
            at: 1,
            down: 2,
            up: 3,
            apps: vec![AppRate {
                path: r"C:\a.exe".into(),
                name: "A".into(),
                down: 2,
                up: 3,
                packets_down: 1,
                packets_up: 1,
                hosts: vec![HostRate {
                    host: "cdn.x.com".into(),
                    service: Some("X".into()),
                    domain: Some("x.com".into()),
                    down: 2,
                    up: 3,
                    packets_down: 1,
                    packets_up: 1,
                }],
            }],
        }))
        .unwrap();
        assert_eq!(
            json,
            r#"{"kind":"tick","at":1,"down":2,"up":3,"apps":[{"path":"C:\\a.exe","name":"A","down":2,"up":3,"packetsDown":1,"packetsUp":1,"hosts":[{"host":"cdn.x.com","service":"X","domain":"x.com","down":2,"up":3,"packetsDown":1,"packetsUp":1}]}]}"#
        );
    }
}
