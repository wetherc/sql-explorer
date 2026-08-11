use futures_util::task::AtomicWaker;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::Waker;

/// A handle that asks the server to cancel the request in flight.
///
/// The [`Client`] gives the handle out through
/// [`attention_handle`], and the handle stays valid for the life of the
/// connection. Any thread can call [`signal`] while another task reads the
/// results of a query. The connection then sends an `Attention` packet to the
/// server, and the server stops the request and sends an acknowledgement. The
/// stream of the cancelled request ends with [`Error::Canceled`].
///
/// A signal with no request in flight does nothing.
///
/// [`Client`]: struct.Client.html
/// [`attention_handle`]: struct.Client.html#method.attention_handle
/// [`signal`]: #method.signal
/// [`Error::Canceled`]: enum.Error.html
#[derive(Debug, Default)]
pub struct AttentionHandle {
    wanted: AtomicBool,
    waker: AtomicWaker,
}

impl AttentionHandle {
    /// Asks the connection to cancel the request in flight.
    pub fn signal(&self) {
        self.wanted.store(true, Ordering::SeqCst);
        self.waker.wake();
    }

    /// Stores the waker of the task that reads from the connection, so that a
    /// later signal wakes the task.
    pub(crate) fn register(&self, waker: &Waker) {
        self.waker.register(waker);
    }

    /// True when a signal waits for the connection to act on it.
    pub(crate) fn wanted(&self) -> bool {
        self.wanted.load(Ordering::SeqCst)
    }

    /// Removes a signal, either because the attention packet went out or
    /// because no request was in flight.
    pub(crate) fn clear(&self) {
        self.wanted.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use crate::tds::codec::{Encode, PacketHeader, PacketStatus, PacketType, PreloginMessage};
    use crate::{AuthMethod, Client, Config, EncryptionLevel, Error};
    use bytes::BytesMut;
    use futures_util::TryStreamExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
    use tokio_util::compat::TokioAsyncReadCompatExt;

    /// The `Attention` flag of a `DONE` token.
    const DONE_ATTENTION: u16 = 1 << 5;

    fn config() -> Config {
        let mut config = Config::new();
        config.authentication(AuthMethod::sql_server("user", "password"));
        config.encryption(EncryptionLevel::NotSupported);
        config
    }

    /// Reads one client message and gives back the packet type of its first
    /// packet with the payload of every packet of the message.
    async fn read_message(server: &mut DuplexStream) -> (u8, Vec<u8>) {
        let mut first_type = None;
        let mut payload = Vec::new();

        loop {
            let mut header = [0u8; 8];
            server.read_exact(&mut header).await.unwrap();

            let length = u16::from_be_bytes([header[2], header[3]]) as usize;
            let mut body = vec![0u8; length - 8];
            server.read_exact(&mut body).await.unwrap();

            first_type.get_or_insert(header[0]);
            payload.extend(body);

            // Bit 0 of the status byte marks the end of the message.
            if header[1] & 1 == 1 {
                return (first_type.unwrap(), payload);
            }
        }
    }

    /// Writes one server packet with the given type, status and payload.
    async fn write_packet(server: &mut DuplexStream, status: PacketStatus, payload: &[u8]) {
        let mut header = PacketHeader::new(payload.len() + 8, 0);
        header.set_type(PacketType::TabularResult);
        header.set_status(status);

        let mut buf = BytesMut::new();
        header.encode(&mut buf).unwrap();
        buf.extend_from_slice(payload);

        server.write_all(&buf).await.unwrap();
    }

    /// Writes one complete server message.
    async fn write_message(server: &mut DuplexStream, payload: &[u8]) {
        write_packet(server, PacketStatus::EndOfMessage, payload).await;
    }

    /// A `DONE` token with the given status flags and row count.
    fn done_token(status: u16, rows: u64) -> Vec<u8> {
        let mut buf = vec![0xFD];
        buf.extend_from_slice(&status.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.extend_from_slice(&rows.to_le_bytes());
        buf
    }

    /// A `COLMETADATA` token with one `int` column and one `ROW` token that
    /// carries the given value.
    fn one_int_row(value: i32) -> Vec<u8> {
        let mut buf = vec![0x81];
        buf.extend_from_slice(&1u16.to_le_bytes()); // one column
        buf.extend_from_slice(&0u32.to_le_bytes()); // user type
        buf.extend_from_slice(&0u16.to_le_bytes()); // flags
        buf.push(0x38); // the fixed length type `int`
        buf.push(1); // the length of the column name in characters
        buf.extend_from_slice(&('a' as u16).to_le_bytes());
        buf.push(0xD1); // a row token
        buf.extend_from_slice(&value.to_le_bytes());
        buf
    }

    /// Answers the prelogin and the login of a connecting client.
    async fn accept_login(server: &mut DuplexStream) {
        let (ty, _) = read_message(server).await;
        assert_eq!(ty, PacketType::PreLogin as u8);

        let mut prelogin = BytesMut::new();
        PreloginMessage::new().encode(&mut prelogin).unwrap();
        write_message(server, &prelogin).await;

        let (ty, _) = read_message(server).await;
        assert_eq!(ty, PacketType::TDSv7Login as u8);

        write_message(server, &done_token(0, 0)).await;
    }

    #[tokio::test]
    async fn a_signal_stops_the_request_and_keeps_the_connection() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let (give_handle, take_handle) = tokio::sync::oneshot::channel();

        let server_task = tokio::spawn(async move {
            accept_login(&mut server).await;
            let handle: std::sync::Arc<super::AttentionHandle> = take_handle.await.unwrap();

            // The first request runs with no answer until the cancel arrives.
            let (ty, _) = read_message(&mut server).await;
            assert_eq!(ty, PacketType::Rpc as u8);

            handle.signal();

            let (ty, payload) = read_message(&mut server).await;
            assert_eq!(ty, PacketType::AttentionSignal as u8);
            assert!(payload.is_empty());

            write_message(&mut server, &done_token(DONE_ATTENTION, 0)).await;

            // The connection takes a second request.
            let (ty, _) = read_message(&mut server).await;
            assert_eq!(ty, PacketType::Rpc as u8);

            write_message(&mut server, &done_token(0, 0)).await;
        });

        let mut client = Client::connect(config(), client_end.compat())
            .await
            .unwrap();
        give_handle.send(client.attention_handle()).unwrap();

        let stopped = client.execute("SELECT 1", &[]).await;
        assert!(matches!(stopped, Err(Error::Canceled)));

        let answered = client.execute("SELECT 2", &[]).await;
        assert!(answered.is_ok());

        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn a_late_acknowledgement_drains_before_the_next_request() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);
        let (give_handle, take_handle) = tokio::sync::oneshot::channel();

        let server_task = tokio::spawn(async move {
            accept_login(&mut server).await;
            let handle: std::sync::Arc<super::AttentionHandle> = take_handle.await.unwrap();

            let (ty, _) = read_message(&mut server).await;
            assert_eq!(ty, PacketType::Rpc as u8);

            // The request finishes before the attention packet arrives. The
            // acknowledgement then comes as a message of its own.
            handle.signal();
            write_message(&mut server, &done_token(0, 0)).await;

            let (ty, _) = read_message(&mut server).await;
            assert_eq!(ty, PacketType::AttentionSignal as u8);

            write_message(&mut server, &done_token(DONE_ATTENTION, 0)).await;

            // The next request must arrive after the acknowledgement went out.
            let (ty, _) = read_message(&mut server).await;
            assert_eq!(ty, PacketType::Rpc as u8);

            write_message(&mut server, &done_token(0, 0)).await;
        });

        let mut client = Client::connect(config(), client_end.compat())
            .await
            .unwrap();
        give_handle.send(client.attention_handle()).unwrap();

        // The request ran to its end on the server, so it succeeds.
        let finished = client.execute("SELECT 1", &[]).await;
        assert!(finished.is_ok());

        let answered = client.execute("SELECT 2", &[]).await;
        assert!(answered.is_ok());

        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn a_signal_with_no_request_in_flight_does_nothing() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);

        let server_task = tokio::spawn(async move {
            accept_login(&mut server).await;

            // The first message after the idle signal must be the request,
            // and never an attention packet.
            let (ty, _) = read_message(&mut server).await;
            assert_eq!(ty, PacketType::Rpc as u8);

            write_message(&mut server, &done_token(0, 0)).await;
        });

        let mut client = Client::connect(config(), client_end.compat())
            .await
            .unwrap();

        client.attention_handle().signal();

        let answered = client.execute("SELECT 1", &[]).await;
        assert!(answered.is_ok());

        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn a_stopped_query_stream_ends_with_the_cancel_error() {
        let (client_end, mut server) = tokio::io::duplex(64 * 1024);

        let server_task = tokio::spawn(async move {
            accept_login(&mut server).await;

            let (ty, _) = read_message(&mut server).await;
            assert_eq!(ty, PacketType::SQLBatch as u8);

            // The response starts with a resultset and one row, and then the
            // query keeps running.
            write_packet(&mut server, PacketStatus::NormalMessage, &one_int_row(42)).await;

            let (ty, _) = read_message(&mut server).await;
            assert_eq!(ty, PacketType::AttentionSignal as u8);

            // The truncated response ends with the acknowledgement.
            write_packet(
                &mut server,
                PacketStatus::EndOfMessage,
                &done_token(DONE_ATTENTION, 0),
            )
            .await;

            let (ty, _) = read_message(&mut server).await;
            assert_eq!(ty, PacketType::Rpc as u8);

            write_message(&mut server, &done_token(0, 0)).await;
        });

        let mut client = Client::connect(config(), client_end.compat())
            .await
            .unwrap();
        let handle = client.attention_handle();

        let mut stream = client.simple_query("SELECT a FROM b").await.unwrap();

        let metadata = stream.try_next().await.unwrap().unwrap();
        assert_eq!("a", metadata.as_metadata().unwrap().columns()[0].name());

        let row = stream.try_next().await.unwrap().unwrap();
        assert_eq!(Some(42), row.as_row().unwrap().get::<i32, _>(0));

        handle.signal();

        let end = stream.try_next().await;
        assert!(matches!(end, Err(Error::Canceled)));

        drop(stream);

        let answered = client.execute("SELECT 1", &[]).await;
        assert!(answered.is_ok());

        server_task.await.unwrap();
    }
}
