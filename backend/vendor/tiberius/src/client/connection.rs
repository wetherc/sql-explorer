#[cfg(any(
    feature = "rustls",
    feature = "native-tls",
    feature = "vendored-openssl"
))]
use crate::client::{tls::TlsPreloginWrapper, tls_stream::create_tls_stream};
use crate::{
    client::{attention::AttentionHandle, tls::MaybeTlsStream, AuthMethod, Config},
    tds::{
        codec::{
            self, Encode, LoginMessage, Packet, PacketCodec, PacketHeader, PacketStatus,
            PacketType, PreloginMessage, TokenDone,
        },
        stream::TokenStream,
        Context, HEADER_BYTES,
    },
    EncryptionLevel, SqlReadBytes,
};
use asynchronous_codec::Framed;
use bytes::BytesMut;
#[cfg(any(windows, feature = "integrated-auth-gssapi"))]
use codec::TokenSspi;
use futures_util::io::{AsyncRead, AsyncWrite};
use futures_util::ready;
use futures_util::sink::{Sink, SinkExt};
use futures_util::stream::{Stream, TryStream, TryStreamExt};
#[cfg(all(unix, feature = "integrated-auth-gssapi"))]
use libgssapi::{
    context::{ClientCtx, CtxFlags},
    credential::{Cred, CredUsage},
    name::Name,
    oid::{OidSet, GSS_MECH_KRB5, GSS_NT_KRB5_PRINCIPAL},
};
use pretty_hex::*;
#[cfg(all(unix, feature = "integrated-auth-gssapi"))]
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{cmp, fmt::Debug, future::Future, io, pin::Pin, task};
use task::Poll;
use tracing::{event, Level};
#[cfg(all(windows, feature = "winauth"))]
use winauth::{windows::NtlmSspiBuilder, NextBytes};

/// Runs a blocking call of the GSSAPI library.
///
/// The first `gss_init_sec_context` asks the KDC for a service ticket. That
/// call makes DNS lookups and network I/O, and a slow KDC keeps it waiting for
/// the krb5 time limits. Inline on an async worker thread, the call stops all
/// other tasks of that thread, and a time limit around the connect cannot end
/// the wait, because the future never yields.
///
/// With the `tokio` feature and inside a tokio runtime, the call runs on the
/// blocking pool of the runtime. When the future is dropped during the wait,
/// the call continues on that pool until it ends. Without the feature, or
/// outside a tokio runtime, the call runs inline, because the crate does not
/// otherwise know which executor drives it.
#[cfg(all(unix, feature = "integrated-auth-gssapi"))]
async fn run_gssapi<T, F>(call: F) -> crate::Result<T>
where
    F: FnOnce() -> crate::Result<T> + Send + 'static,
    T: Send + 'static,
{
    #[cfg(feature = "tokio")]
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        return match runtime.spawn_blocking(call).await {
            Ok(result) => result,
            Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
            Err(error) => Err(crate::Error::Gssapi(error.to_string())),
        };
    }
    call()
}

/// Awaits one GSSAPI call and keeps `flag` true while the call waits.
///
/// The flag goes back to false only when the call returns. When a time limit
/// drops the future during the wait, the flag stays true, so the caller of
/// the connect can tell where the limit passed. See [`Config::watch_gssapi`].
#[cfg_attr(not(all(unix, feature = "integrated-auth-gssapi")), allow(dead_code))]
async fn watched<F: Future>(flag: Option<&AtomicBool>, call: F) -> F::Output {
    if let Some(flag) = flag {
        flag.store(true, Ordering::SeqCst);
    }
    let output = call.await;
    if let Some(flag) = flag {
        flag.store(false, Ordering::SeqCst);
    }
    output
}

/// A `Connection` is an abstraction between the [`Client`] and the server. It
/// can be used as a `Stream` to fetch [`Packet`]s from and to `send` packets
/// splitting them to the negotiated limit automatically.
///
/// `Connection` is not meant to use directly, but as an abstraction layer for
/// the numerous `Stream`s for easy packet handling.
///
/// [`Client`]: struct.Encode.html
/// [`Packet`]: ../protocol/codec/struct.Packet.html
pub(crate) struct Connection<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    transport: Framed<MaybeTlsStream<S>, PacketCodec>,
    flushed: bool,
    context: Context,
    buf: BytesMut,
    attention: Arc<AttentionHandle>,
    attention_flushing: bool,
    attention_pending: bool,
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> Debug for Connection<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("transport", &"Framed<..>")
            .field("flushed", &self.flushed)
            .field("context", &self.context)
            .field("buf", &self.buf.as_ref().hex_dump())
            .finish()
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> Connection<S> {
    /// Creates a new connection
    pub(crate) async fn connect(config: Config, tcp_stream: S) -> crate::Result<Connection<S>> {
        let context = {
            let mut context = Context::new();
            context.set_spn(config.get_host(), config.get_port());
            context
        };

        let transport = Framed::new(MaybeTlsStream::Raw(tcp_stream), PacketCodec);

        let mut connection = Self {
            transport,
            context,
            flushed: false,
            buf: BytesMut::new(),
            attention: Arc::new(AttentionHandle::default()),
            attention_flushing: false,
            attention_pending: false,
        };

        let fed_auth_required = matches!(config.auth, AuthMethod::AADToken(_));

        let prelogin = connection
            .prelogin(config.encryption, fed_auth_required)
            .await?;

        let encryption = prelogin.negotiated_encryption(config.encryption)?;

        let connection = connection.tls_handshake(&config, encryption).await?;

        let mut connection = connection
            .login(
                config.auth,
                encryption,
                config.database,
                config.host,
                config.application_name,
                config.readonly,
                prelogin,
                config.gssapi_wait,
            )
            .await?;

        connection.flush_done().await?;

        Ok(connection)
    }

    /// Flush the incoming token stream until receiving `DONE` token.
    async fn flush_done(&mut self) -> crate::Result<TokenDone> {
        TokenStream::new(self).flush_done().await
    }

    #[cfg(any(windows, feature = "integrated-auth-gssapi"))]
    /// Flush the incoming token stream until receiving `SSPI` token.
    async fn flush_sspi(&mut self) -> crate::Result<TokenSspi> {
        TokenStream::new(self).flush_sspi().await
    }

    #[cfg(any(
        feature = "rustls",
        feature = "native-tls",
        feature = "vendored-openssl"
    ))]
    fn post_login_encryption(mut self, encryption: EncryptionLevel) -> Self {
        if let EncryptionLevel::Off = encryption {
            event!(
                Level::WARN,
                "Turning TLS off after a login. All traffic from here on is not encrypted.",
            );

            let Self { transport, .. } = self;
            let tcp = transport.into_inner().into_inner();
            self.transport = Framed::new(MaybeTlsStream::Raw(tcp), PacketCodec);
        }

        self
    }

    #[cfg(not(any(
        feature = "rustls",
        feature = "native-tls",
        feature = "vendored-openssl"
    )))]
    fn post_login_encryption(self, _: EncryptionLevel) -> Self {
        self
    }

    /// Send an item to the wire. Header should define the item type and item should implement
    /// [`Encode`], defining the byte structure for the wire.
    ///
    /// The `send` will split the packet into multiple packets if bigger than
    /// the negotiated packet size, and handle flushing to the wire in an optimal way.
    ///
    /// [`Encode`]: ../protocol/codec/trait.Encode.html
    pub async fn send<E>(&mut self, mut header: PacketHeader, item: E) -> crate::Result<()>
    where
        E: Sized + Encode<BytesMut>,
    {
        // A cancel signal that arrived with no request in flight targets a
        // request that already ended. The new request must not inherit it.
        self.attention.clear();

        self.flushed = false;
        let packet_size = (self.context.packet_size() as usize) - HEADER_BYTES;

        let mut payload = BytesMut::new();
        item.encode(&mut payload)?;

        while !payload.is_empty() {
            let writable = cmp::min(payload.len(), packet_size);
            let split_payload = payload.split_to(writable);

            if payload.is_empty() {
                header.set_status(PacketStatus::EndOfMessage);
            } else {
                header.set_status(PacketStatus::NormalMessage);
            }

            event!(
                Level::TRACE,
                "Sending a packet ({} bytes)",
                split_payload.len() + HEADER_BYTES,
            );

            self.write_to_wire(header, split_payload).await?;
        }

        self.flush_sink().await?;

        Ok(())
    }

    /// Sends a packet of data to the database.
    ///
    /// # Warning
    ///
    /// Please be sure the packet size doesn't exceed the largest allowed size
    /// dictaded by the server.
    pub(crate) async fn write_to_wire(
        &mut self,
        header: PacketHeader,
        data: BytesMut,
    ) -> crate::Result<()> {
        self.flushed = false;

        let packet = Packet::new(header, data);
        self.transport.send(packet).await?;

        Ok(())
    }

    /// Sends all pending packages to the wire.
    pub(crate) async fn flush_sink(&mut self) -> crate::Result<()> {
        self.transport.flush().await
    }

    /// Cleans the packet stream from previous use. It is important to use the
    /// whole stream before using the connection again. Flushing the stream
    /// makes sure we don't have any old data causing undefined behaviour after
    /// previous queries.
    ///
    /// Calling this will slow down the queries if stream is still dirty if all
    /// results are not handled.
    pub async fn flush_stream(&mut self) -> crate::Result<()> {
        self.buf.truncate(0);

        if self.flushed {
            return Ok(());
        }

        while let Some(packet) = self.try_next().await? {
            event!(
                Level::WARN,
                "Flushing unhandled packet from the wire. Please consume your streams!",
            );

            let is_last = packet.is_last();

            if is_last {
                break;
            }
        }

        Ok(())
    }

    /// True if the underlying stream has no more data and is consumed
    /// completely.
    pub fn is_eof(&self) -> bool {
        self.flushed && self.buf.is_empty()
    }

    /// A message sent by the client to set up context for login. The server
    /// responds to a client PRELOGIN message with a message of packet header
    /// type 0x04 and with the packet data containing a PRELOGIN structure.
    ///
    /// This message stream is also used to wrap the TLS handshake payload if
    /// encryption is needed. In this scenario, where PRELOGIN message is
    /// transporting the TLS handshake payload, the packet data is simply the
    /// raw bytes of the TLS handshake payload.
    async fn prelogin(
        &mut self,
        encryption: EncryptionLevel,
        fed_auth_required: bool,
    ) -> crate::Result<PreloginMessage> {
        let mut msg = PreloginMessage::new();
        msg.encryption = encryption;
        msg.fed_auth_required = fed_auth_required;

        let id = self.context.next_packet_id();
        self.send(PacketHeader::pre_login(id), msg).await?;

        let response: PreloginMessage = codec::collect_from(self).await?;
        // threadid (should be empty when sent from server to client)
        debug_assert_eq!(response.thread_id, 0);
        Ok(response)
    }

    /// Defines the login record rules with SQL Server. Authentication with
    /// connection options.
    #[allow(clippy::too_many_arguments)]
    async fn login<'a>(
        mut self,
        auth: AuthMethod,
        encryption: EncryptionLevel,
        db: Option<String>,
        server_name: Option<String>,
        application_name: Option<String>,
        readonly: bool,
        prelogin: PreloginMessage,
        #[cfg_attr(
            not(all(unix, feature = "integrated-auth-gssapi")),
            allow(unused_variables)
        )]
        gssapi_wait: Option<Arc<AtomicBool>>,
    ) -> crate::Result<Self> {
        let mut login_message = LoginMessage::new();

        if let Some(db) = db {
            login_message.db_name(db);
        }

        if let Some(server_name) = server_name {
            login_message.server_name(server_name);
        }

        if let Some(app_name) = application_name {
            login_message.app_name(app_name);
        }

        login_message.readonly(readonly);

        match auth {
            #[cfg(all(windows, feature = "winauth"))]
            AuthMethod::Integrated => {
                let mut client = NtlmSspiBuilder::new()
                    .target_spn(self.context.spn())
                    .build()?;

                login_message.integrated_security(client.next_bytes(None)?);

                let id = self.context.next_packet_id();
                self.send(PacketHeader::login(id), login_message).await?;

                self = self.post_login_encryption(encryption);

                let sspi_bytes = self.flush_sspi().await?;

                match client.next_bytes(Some(sspi_bytes.as_ref()))? {
                    Some(sspi_response) => {
                        event!(Level::TRACE, sspi_response_len = sspi_response.len());

                        let id = self.context.next_packet_id();
                        let header = PacketHeader::login(id);

                        let token = TokenSspi::new(sspi_response);
                        self.send(header, token).await?;
                    }
                    None => unreachable!(),
                }
            }
            #[cfg(all(unix, feature = "integrated-auth-gssapi"))]
            AuthMethod::Integrated => {
                let spn = self.context.spn().to_string();

                let wait = gssapi_wait.as_deref();
                let (mut ctx, init_token) = watched(
                    wait,
                    run_gssapi(move || {
                        let mut s = OidSet::new()?;
                        s.add(&GSS_MECH_KRB5)?;

                        let client_cred = Cred::acquire(None, None, CredUsage::Initiate, Some(&s))?;

                        let mut ctx = ClientCtx::new(
                            Some(client_cred),
                            Name::new(spn.as_bytes(), Some(&GSS_NT_KRB5_PRINCIPAL))?,
                            CtxFlags::GSS_C_MUTUAL_FLAG | CtxFlags::GSS_C_SEQUENCE_FLAG,
                            None,
                        );

                        let init_token =
                            ctx.step(None, None)?.map(|token| Vec::from(token.deref()));
                        Ok((ctx, init_token))
                    }),
                )
                .await?;

                login_message.integrated_security(Some(init_token.unwrap()));

                let id = self.context.next_packet_id();
                self.send(PacketHeader::login(id), login_message).await?;

                self = self.post_login_encryption(encryption);

                let auth_bytes = self.flush_sspi().await?;

                let response = watched(
                    wait,
                    run_gssapi(move || {
                        let response = ctx.step(Some(auth_bytes.as_ref()), None)?;
                        Ok(response.map(|response| Vec::from(response.deref())))
                    }),
                )
                .await?;

                let next_token = match response {
                    Some(response) => {
                        event!(Level::TRACE, response_len = response.len());
                        TokenSspi::new(response)
                    }
                    None => {
                        event!(Level::TRACE, response_len = 0);
                        TokenSspi::new(Vec::new())
                    }
                };

                let id = self.context.next_packet_id();
                let header = PacketHeader::login(id);

                self.send(header, next_token).await?;
            }
            #[cfg(all(windows, feature = "winauth"))]
            AuthMethod::Windows(auth) => {
                let spn = self.context.spn().to_string();
                let builder = winauth::NtlmV2ClientBuilder::new().target_spn(spn);
                let mut client = builder.build(auth.domain, auth.user, auth.password);

                login_message.integrated_security(client.next_bytes(None)?);

                let id = self.context.next_packet_id();
                self.send(PacketHeader::login(id), login_message).await?;

                self = self.post_login_encryption(encryption);

                let sspi_bytes = self.flush_sspi().await?;

                match client.next_bytes(Some(sspi_bytes.as_ref()))? {
                    Some(sspi_response) => {
                        event!(Level::TRACE, sspi_response_len = sspi_response.len());

                        let id = self.context.next_packet_id();
                        let header = PacketHeader::login(id);

                        let token = TokenSspi::new(sspi_response);
                        self.send(header, token).await?;
                    }
                    None => unreachable!(),
                }
            }
            AuthMethod::None => {
                let id = self.context.next_packet_id();
                self.send(PacketHeader::login(id), login_message).await?;
                self = self.post_login_encryption(encryption);
            }
            AuthMethod::SqlServer(auth) => {
                login_message.user_name(auth.user());
                login_message.password(auth.password());

                let id = self.context.next_packet_id();
                self.send(PacketHeader::login(id), login_message).await?;
                self = self.post_login_encryption(encryption);
            }
            AuthMethod::AADToken(token) => {
                login_message.aad_token(token, prelogin.fed_auth_required, prelogin.nonce);
                let id = self.context.next_packet_id();
                self.send(PacketHeader::login(id), login_message).await?;
                self = self.post_login_encryption(encryption);
            }
        }

        Ok(self)
    }

    /// Implements the TLS handshake with the SQL Server.
    #[cfg(any(
        feature = "rustls",
        feature = "native-tls",
        feature = "vendored-openssl"
    ))]
    async fn tls_handshake(
        self,
        config: &Config,
        encryption: EncryptionLevel,
    ) -> crate::Result<Self> {
        if encryption != EncryptionLevel::NotSupported {
            event!(Level::INFO, "Performing a TLS handshake");

            let Self {
                transport,
                context,
                attention,
                ..
            } = self;
            let mut stream = match transport.into_inner() {
                MaybeTlsStream::Raw(tcp) => {
                    create_tls_stream(config, TlsPreloginWrapper::new(tcp)).await?
                }
                _ => unreachable!(),
            };

            stream.get_mut().handshake_complete();
            event!(Level::INFO, "TLS handshake successful");

            let transport = Framed::new(MaybeTlsStream::Tls(stream), PacketCodec);

            Ok(Self {
                transport,
                context,
                flushed: false,
                buf: BytesMut::new(),
                attention,
                attention_flushing: false,
                attention_pending: false,
            })
        } else {
            event!(
                Level::WARN,
                "TLS encryption is not enabled. All traffic including the login credentials are not encrypted."
            );

            Ok(self)
        }
    }

    /// Implements the TLS handshake with the SQL Server.
    #[cfg(not(any(
        feature = "rustls",
        feature = "native-tls",
        feature = "vendored-openssl"
    )))]
    async fn tls_handshake(self, _: &Config, _: EncryptionLevel) -> crate::Result<Self> {
        event!(
            Level::WARN,
            "TLS encryption is not enabled. All traffic including the login credentials are not encrypted."
        );

        Ok(self)
    }

    pub(crate) async fn close(mut self) -> crate::Result<()> {
        self.transport.close().await
    }

    /// Gives the shared handle that asks the server to cancel the request in
    /// flight.
    pub(crate) fn attention_handle(&self) -> Arc<AttentionHandle> {
        self.attention.clone()
    }

    /// Records the acknowledgement of an attention packet. The token stream
    /// calls this when a `DONE` token carries the attention flag.
    pub(crate) fn attention_acknowledged(&mut self) {
        self.attention_pending = false;
    }

    /// True when an attention packet went out and its acknowledgement did not
    /// arrive yet. The read of the stream then continues with the next
    /// message of the server, which brings the acknowledgement.
    pub(crate) fn await_attention_ack(&mut self) -> bool {
        if self.attention_pending {
            self.flushed = false;
        }
        self.attention_pending
    }

    /// Reads and discards tokens until the acknowledgement of an attention
    /// packet arrives. A new request must wait for the acknowledgement of the
    /// old one, or it would read the acknowledgement as its own response.
    pub(crate) async fn drain_attention_ack(&mut self) -> crate::Result<()> {
        if !self.attention_pending {
            return Ok(());
        }

        // The acknowledgement comes as a message of its own when the response
        // finished before the attention packet reached the server.
        self.flushed = false;

        let mut stream = TokenStream::new(self).try_unfold();

        let outcome = loop {
            match stream.try_next().await {
                // A token of the stopped request. Read past it.
                Ok(Some(_)) => (),
                Ok(None) => {
                    break Err(crate::Error::Protocol(
                        "the acknowledgement of the attention packet never arrived".into(),
                    ))
                }
                // The token stream turns the acknowledgement into this error.
                Err(crate::Error::Canceled) => break Ok(()),
                Err(e) => break Err(e),
            }
        };
        drop(stream);

        // The acknowledgement is the last token of its message. When it stood
        // in the buffer already, no packet was read, so the flag of the end
        // would stay false and `flush_stream` would wait for a packet that
        // never comes.
        if outcome.is_ok() {
            self.flushed = true;
        }
        outcome
    }

    /// Sends an attention packet when a signal asks for one, and drives the
    /// write of that packet to completion. The caller polls the transport for
    /// reads right after, so both halves make progress on every poll.
    fn poll_attention(&mut self, cx: &mut task::Context<'_>) -> crate::Result<()> {
        self.attention.register(cx.waker());

        if self.attention.wanted() {
            if self.flushed || self.attention_pending {
                // The request already ended, or an attention packet already
                // went out. A second packet would confuse the bookkeeping of
                // the acknowledgement.
                self.attention.clear();
            } else {
                match Pin::new(&mut self.transport).poll_ready(cx) {
                    Poll::Ready(Ok(())) => {
                        let id = self.context.next_packet_id();
                        let mut header = PacketHeader::new(HEADER_BYTES, id);
                        header.set_type(PacketType::AttentionSignal);
                        header.set_status(PacketStatus::EndOfMessage);

                        event!(Level::DEBUG, "Sending an attention packet");

                        Pin::new(&mut self.transport)
                            .start_send(Packet::new(header, BytesMut::new()))?;

                        self.attention.clear();
                        self.attention_pending = true;
                        self.attention_flushing = true;
                    }
                    Poll::Ready(Err(e)) => return Err(e),
                    // The sink holds too much data. The signal stays set and
                    // the next poll tries again.
                    Poll::Pending => (),
                }
            }
        }

        if self.attention_flushing {
            match Pin::new(&mut self.transport).poll_flush(cx) {
                Poll::Ready(Ok(())) => self.attention_flushing = false,
                Poll::Ready(Err(e)) => return Err(e),
                // The flush registered its own waker and continues on the
                // next poll. The read below makes progress in the meantime.
                Poll::Pending => (),
            }
        }

        Ok(())
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> Stream for Connection<S> {
    type Item = crate::Result<Packet>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        if let Err(e) = this.poll_attention(cx) {
            return Poll::Ready(Some(Err(e)));
        }

        match ready!(this.transport.try_poll_next_unpin(cx)) {
            Some(Ok(packet)) => {
                this.flushed = packet.is_last();
                Poll::Ready(Some(Ok(packet)))
            }
            Some(Err(e)) => Poll::Ready(Some(Err(e))),
            None => Poll::Ready(None),
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> futures_util::io::AsyncRead for Connection<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let mut this = self.get_mut();
        let size = buf.len();

        if this.buf.len() < size {
            while let Some(item) = ready!(Pin::new(&mut this).try_poll_next(cx)) {
                match item {
                    Ok(packet) => {
                        let (_, payload) = packet.into_parts();
                        this.buf.extend(payload);

                        if this.buf.len() >= size {
                            break;
                        }
                    }
                    Err(e) => {
                        return Poll::Ready(Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            e.to_string(),
                        )))
                    }
                }
            }

            // Got EOF before having all the data.
            if this.buf.len() < size {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "No more packets in the wire",
                )));
            }
        }

        buf.copy_from_slice(this.buf.split_to(size).as_ref());
        Poll::Ready(Ok(size))
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> SqlReadBytes for Connection<S> {
    /// Hex dump of the current buffer.
    fn debug_buffer(&self) {
        dbg!(self.buf.as_ref().hex_dump());
    }

    /// The current execution context.
    fn context(&self) -> &Context {
        &self.context
    }

    /// A mutable reference to the current execution context.
    fn context_mut(&mut self) -> &mut Context {
        &mut self.context
    }
}

#[cfg(test)]
mod watch_tests {
    use super::watched;
    use futures_util::FutureExt;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn the_flag_is_true_while_the_call_waits_and_false_after_it() {
        let flag = AtomicBool::new(false);
        let ready = std::cell::Cell::new(false);
        let answer = futures_util::future::poll_fn(|_| match ready.get() {
            true => std::task::Poll::Ready(7),
            false => std::task::Poll::Pending,
        });
        let mut call = Box::pin(watched(Some(&flag), answer));
        assert!(call.as_mut().now_or_never().is_none());
        assert!(flag.load(Ordering::SeqCst));
        ready.set(true);
        assert_eq!(call.now_or_never(), Some(7));
        assert!(!flag.load(Ordering::SeqCst));
    }

    #[test]
    fn a_call_that_is_dropped_during_the_wait_leaves_the_flag_true() {
        let flag = AtomicBool::new(false);
        let mut call = Box::pin(watched(Some(&flag), futures_util::future::pending::<()>()));
        assert!(call.as_mut().now_or_never().is_none());
        drop(call);
        assert!(flag.load(Ordering::SeqCst));
    }

    #[test]
    fn a_call_without_a_flag_runs_as_it_is() {
        assert_eq!(watched(None, async { 3 }).now_or_never(), Some(3));
    }

    #[test]
    fn the_config_gives_the_flag_to_the_login() {
        let mut config = crate::Config::new();
        assert!(config.gssapi_wait.is_none());
        let flag = std::sync::Arc::new(AtomicBool::new(false));
        config.watch_gssapi(flag.clone());
        assert!(std::sync::Arc::ptr_eq(
            config.gssapi_wait.as_ref().unwrap(),
            &flag
        ));
    }
}

#[cfg(all(test, unix, feature = "integrated-auth-gssapi"))]
mod tests {
    use super::run_gssapi;
    use futures_util::FutureExt;

    #[test]
    fn outside_a_runtime_the_call_runs_inline() {
        let caller = std::thread::current().id();
        let ran_on = run_gssapi(|| Ok(std::thread::current().id()))
            .now_or_never()
            .unwrap()
            .unwrap();
        assert_eq!(ran_on, caller);
    }

    #[cfg(feature = "tokio")]
    #[tokio::test]
    async fn inside_a_tokio_runtime_the_call_leaves_the_worker_thread() {
        let caller = std::thread::current().id();
        let ran_on = run_gssapi(|| Ok(std::thread::current().id()))
            .await
            .unwrap();
        assert_ne!(ran_on, caller);

        let error = run_gssapi::<(), _>(|| Err(crate::Error::Gssapi("no ticket".into())))
            .await
            .unwrap_err();
        assert_eq!(error, crate::Error::Gssapi("no ticket".into()));
    }

    #[cfg(feature = "tokio")]
    #[tokio::test]
    #[should_panic(expected = "the call panicked")]
    async fn a_panic_of_the_call_reaches_the_caller() {
        let _ = run_gssapi::<(), _>(|| panic!("the call panicked")).await;
    }
}
