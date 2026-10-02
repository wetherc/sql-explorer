use crate::config::{SslMode, SslNegotiation};
use crate::tls::TlsConnect;
use crate::{Error, connect_tls};
use bytes::BytesMut;
use postgres_protocol::message::frontend;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub async fn cancel_query_raw<S, T>(
    stream: S,
    mode: SslMode,
    negotiation: SslNegotiation,
    tls: T,
    has_hostname: bool,
    process_id: i32,
    secret_key: i32,
) -> Result<(), Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
    T: TlsConnect<S>,
{
    let mut stream = connect_tls::connect_tls(stream, mode, negotiation, tls, has_hostname).await?;

    let mut buf = BytesMut::new();
    frontend::cancel_request(process_id, secret_key, &mut buf);

    stream.write_all(&buf).await.map_err(Error::io)?;
    stream.flush().await.map_err(Error::io)?;
    stream.shutdown().await.map_err(Error::io)?;

    // The server closes the socket after it sends the signal of the cancel to
    // the session. A statement that the client sends before that moment can
    // receive the signal in place of the statement that the cancel was for.
    // The read waits for the close, as libpq does. The server sends no data,
    // and an error of the read also tells that the socket is closed.
    let mut rest = [0u8; 64];
    while let Ok(read) = stream.read(&mut rest).await {
        if read == 0 {
            break;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NoTls;
    use crate::config::{SslMode, SslNegotiation};
    use std::time::Duration;

    #[tokio::test]
    async fn the_cancel_ends_when_the_server_closes_the_socket() {
        let (client, mut server) = tokio::io::duplex(64);
        let cancel = tokio::spawn(cancel_query_raw(
            client,
            SslMode::Disable,
            SslNegotiation::Postgres,
            NoTls,
            false,
            7,
            11,
        ));

        let mut request = [0u8; 16];
        server.read_exact(&mut request).await.unwrap();
        assert_eq!(&request[8..12], &7i32.to_be_bytes());
        assert_eq!(&request[12..16], &11i32.to_be_bytes());

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!cancel.is_finished());

        drop(server);
        cancel.await.unwrap().unwrap();
    }
}
