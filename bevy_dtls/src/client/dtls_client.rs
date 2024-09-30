use std::{net::IpAddr, sync::Arc, time::Duration};
use anyhow::{anyhow, bail};
use bevy::{
    prelude::*, 
    tasks::futures_lite::future
};
use bytes::{Bytes, BytesMut};
use tokio::{
    net::UdpSocket as TokioUdpSocket, 
    runtime::{self, Runtime},
    select,
    sync::mpsc::{
        unbounded_channel as tokio_channel, 
        UnboundedSender as TokioTx,
        UnboundedReceiver as TokioRx,
        error::TryRecvError
    }, 
    task::JoinHandle,
    time::timeout
};
use webrtc_dtls::conn::DTLSConn;
use webrtc_util::Conn;
use super::cert_option::ClientCertOption;

pub struct DtlsClientConfig {
    pub server_addr: IpAddr,
    pub server_port: u16,
    pub client_addr: IpAddr,
    pub client_port: u16,
    pub cert_option: ClientCertOption
}

impl DtlsClientConfig {
    async fn connect(self) 
    -> anyhow::Result<Arc<impl Conn + Sync + Send>> {
        let socket = TokioUdpSocket::bind(
            (self.client_addr, self.client_port)
        )
        .await?;
        socket.connect(
            (self.server_addr, self.server_port)
        )
        .await?;
        debug!("connecting to {}", self.server_addr);

        let dtls_conn = DTLSConn::new(
            Arc::new(socket), 
            self.cert_option.to_dtls_config()?, 
            true, 
            None
        )
        .await?;

        Ok(Arc::new(dtls_conn))
    }
}

pub struct DtlsClientHealth {
    pub sender: Option<anyhow::Result<()>>,
    pub recver: Option<anyhow::Result<()>>,
    pub closed: bool
}

pub enum DtlsClientTimeout {
    Send(Bytes)
}

struct DtlsClientClose;

struct DtlsClientSender {
    conn: Arc<dyn Conn + Sync + Send>,
    timeout_secs: u64,
    send_rx: TokioRx<Bytes>,
    timeout_tx: TokioTx<DtlsClientTimeout>,
    close_rx: TokioRx<DtlsClientClose>
}

impl DtlsClientSender {
    #[inline]
    fn new(conn: Arc<dyn Conn + Send + Sync>, timeout_secs: u64)
    -> (
        TokioTx<Bytes>, 
        TokioRx<DtlsClientTimeout>, 
        TokioTx<DtlsClientClose>, 
        Self
    ) {
        let (send_tx, send_rx) = tokio_channel::<Bytes>();
        let (timeout_tx, timeout_rx) = tokio_channel::<DtlsClientTimeout>();
        let(close_tx, close_rx) = tokio_channel::<DtlsClientClose>();
    
        (send_tx, timeout_rx, close_tx, Self{
            conn,
            timeout_secs,
            send_rx,
            timeout_tx,
            close_rx,
        })
    }

    #[inline]
    fn timeout_secs(&self) -> Duration {
        Duration::from_secs(self.timeout_secs)
    }

    async fn send_loop(mut self)-> anyhow::Result<()> {
        let result = loop {
            select! {
                biased;

                Some(_) = self.close_rx.recv() => break Ok(()),
                Some(msg) = self.send_rx.recv() => {
                    match timeout(
                        self.timeout_secs(), 
                        self.conn.send(&msg)
                    ).await {
                        Ok(r) => {
                            match r {
                                Ok(n) => trace!("sent {n} bytes"),
                                Err(e) => break Err(anyhow!(e))
                            }
                        }
                        Err(_) => {
                            if let Err(e) = self.timeout_tx.send(
                                DtlsClientTimeout::Send(msg)
                            ) {
                                break Err(anyhow!(e));
                            }
                        }
                    }
                }
                else => {
                    warn!("close send tx is closed before rx is closed");
                    break Ok(());
                }
            }
        };

        self.conn.close().await?;
        debug!("dtls client send loop is closed");
        result
    }
}

struct DtlsClientRecver {
    conn: Arc<dyn Conn + Sync + Send>,
    buf_size: usize,
    recv_tx: TokioTx<Bytes>,
    close_rx: TokioRx<DtlsClientClose>
}

impl DtlsClientRecver {
    #[inline]
    fn new(conn: Arc<dyn Conn + Sync + Send>, buf_size: usize)
    -> (TokioRx<Bytes>, TokioTx<DtlsClientClose>, Self) {
        let (recv_tx, recv_rx) = tokio_channel::<Bytes>();
        let (close_tx, close_rx) = tokio_channel::<DtlsClientClose>();

        (recv_rx, close_tx, Self{
            conn,
            buf_size,
            recv_tx,
            close_rx,
        })
    }

    async fn recv_loop(mut self) -> anyhow::Result<()> {
        let mut buf = BytesMut::zeroed(self.buf_size);

        let result = loop {
            let n = select! {
                biased;

                Some(_) = self.close_rx.recv() => break Ok(()),
                r = self.conn.recv(&mut buf) => {
                    match r {
                        Ok(n) => n,
                        Err(e) => break Err(anyhow!(e))
                    }
                }
                else => {
                    warn!("close recv tx is closed before rx is closed");
                    break Ok(());
                }
            };

            let receved = buf.split_to(n)
            .freeze();
            if let Err(e) = self.recv_tx.send(receved) {
                break Err(anyhow!(e));
            }

            buf.resize(self.buf_size, 0);
            trace!("received {n}bytes");
        };

        self.conn.close().await?;
        debug!("dtls client recv loop is closed");
        result
    }
}

#[derive(Resource)]
pub struct DtlsClient {
    runtime: Arc<Runtime>,

    conn: Option<Arc<dyn Conn + Sync + Send>>,
    is_running: bool,

    send_timeout_secs: u64,
    send_handle: Option<JoinHandle<anyhow::Result<()>>>,
    send_tx: Option<TokioTx<Bytes>>,
    send_timeout_rx: Option<TokioRx<DtlsClientTimeout>>,
    close_send_tx: Option<TokioTx<DtlsClientClose>>,

    recv_handle: Option<JoinHandle<anyhow::Result<()>>>,
    recv_buf_size: usize,
    recv_rx: Option<TokioRx<Bytes>>,
    close_recv_tx: Option<TokioTx<DtlsClientClose>>
}

impl DtlsClient {
    #[inline]
    pub fn new(recv_buf_size: usize, send_timeout_secs: u64) 
    -> anyhow::Result<Self> {
        let rt = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?; 

        Ok(Self{
            runtime: Arc::new(rt),

            conn: None,
            is_running: false,

            send_timeout_secs,
            send_handle: None,
            send_tx: None,
            send_timeout_rx: None,
            close_send_tx: None,
            
            recv_handle: None,
            recv_buf_size,
            recv_rx: None,
            close_recv_tx: None
        })
    }

    #[inline]
    pub fn is_closed(&self) -> bool {
        // set closed by health check
        !self.is_running 
        && self.conn.is_none() 
        && self.recv_handle.is_none()
        && self.send_handle.is_none()

        // set closed by calling disconnect
        && self.send_tx.is_none()
        && self.send_timeout_rx.is_none()
        && self.close_send_tx.is_none()
        && self.recv_rx.is_none()
        && self.close_recv_tx.is_none()
    }

    #[inline]
    pub fn start(&mut self, config: DtlsClientConfig) 
    -> anyhow::Result<()> {
        if !self.is_closed() {
            bail!("dtls client is not closed");
        }

        self.start_connect(config)?;
        self.start_send_loop()?;
        self.start_recv_loop()
    }

    pub fn send(&self, message: Bytes) -> anyhow::Result<()> {
        let Some(ref send_tx) = self.send_tx else {
            bail!("send tx is None");
        };

        if let Err(e) = send_tx.send(message) {
            bail!("conn is not started or disconnected: {e}");
        }
        Ok(())
    }

    pub fn recv(&mut self) -> Option<Bytes> {
        let Some(ref mut recv_rx) = self.recv_rx else {
            return None;
        };

        match recv_rx.try_recv() {
            Ok(b) => Some(b),
            Err(e) => {
                if matches!(e, TryRecvError::Disconnected) {
                    warn!("recv rx is closed before set to None: {e}");
                }
                None
            }
        }
    }

    pub fn timeout_check(&mut self) 
    -> std::result::Result<(), DtlsClientTimeout> {
        let Some(ref mut timeout_rx) = self.send_timeout_rx else {
            return Ok(());
        };

        match timeout_rx.try_recv() {
            Ok(t) => Err(t),
            Err(e) => {
                if matches!(e, TryRecvError::Disconnected) {
                    warn!("send timeout rx is closed before set to None: {e}");
                }
                Ok(())
            }
        }
    }

    #[inline]
    pub fn health_check(&mut self) -> DtlsClientHealth {
        let sender_health = self.health_check_send_loop();
        let recver_health = self.health_check_recv_loop();
        let closed = self.is_running
        && self.send_handle.is_none()
        && self.recv_handle.is_none();

        if closed {
            self.conn = None;
            self.is_running = false;
        }
        
        DtlsClientHealth{
            sender: sender_health,
            recver: recver_health,
            closed
        }
    }

    #[inline]
    pub fn disconnect(&mut self) {
        self.close_send_loop();
        self.close_recv_loop();
    }

    fn start_connect(&mut self, config: DtlsClientConfig) 
    -> anyhow::Result<()> {
        let conn = future::block_on(self.runtime.spawn(
            config.connect()
        ))??;
        self.conn = Some(conn);
        debug!("dtls client has connected");
        Ok(())
    }

    fn start_send_loop(&mut self) -> anyhow::Result<()> {
        if self.send_handle.is_some() {
            bail!("join handle already exists, or health_check is not called");
        }
        
        let (
            send_tx, 
            timeout_rx, 
            close_tx, 
            sender
        ) = DtlsClientSender::new(
            match self.conn {
                Some(ref c) => c.clone(),
                None => bail!("conn is none")
            },
            self.send_timeout_secs,
        );

        self.send_tx = Some(send_tx);
        self.send_timeout_rx = Some(timeout_rx);
        self.close_send_tx = Some(close_tx);

        let handle = self.runtime.spawn(sender.send_loop());
        self.send_handle = Some(handle);
        self.is_running = true;

        debug!("send loop has started");
        Ok(())
    }

    fn health_check_send_loop(&mut self) 
    -> Option<anyhow::Result<()>> {
        let handle_ref = self.send_handle.as_ref()?;

        if !handle_ref.is_finished() {
            return None;
        }

        let handle = self.send_handle.take()
        .unwrap();
        match future::block_on(handle) {
            Ok(r) => Some(r),
            Err(e) => Some(Err(anyhow!(e)))
        }
    }

    fn close_send_loop(&mut self) {
        let Some(ref close_send_tx) = self.close_send_tx else {
            return;
        };

        if let Err(e) = close_send_tx.send(DtlsClientClose) {
            warn!("close send tx is closed before set to None: {e}");
        }

        self.close_send_tx = None;
        self.send_timeout_rx = None;
        self.send_tx = None;
    }

    fn start_recv_loop(&mut self) -> anyhow::Result<()> {
        if self.recv_handle.is_some() {
            bail!("join handle already exists, or health_check is not called");
        }
        
        let (recv_rx, close_tx, recver) = DtlsClientRecver::new(
            match self.conn {
                Some(ref c) => c.clone(),
                None => bail!("dtls conn is None")
            },
            self.recv_buf_size
        );
        self.recv_rx = Some(recv_rx);
        self.close_recv_tx = Some(close_tx);

        let handle = self.runtime.spawn(recver.recv_loop());
        self.recv_handle = Some(handle);
        self.is_running = true;

        debug!("recv loop has started");
        Ok(())
    }

    fn health_check_recv_loop(&mut self) 
    -> Option<anyhow::Result<()>> {
        let handle_ref = self.recv_handle.as_ref()?;

        if !handle_ref.is_finished() {
            return None;
        }

        let handle = self.recv_handle.take()
        .unwrap();
        match future::block_on(handle) {
            Ok(r) => Some(r),
            Err(e) => Some(Err(anyhow!(e)))
        }
    }

    fn close_recv_loop(&mut self) {
        let Some(ref close_recv_tx) = self.close_recv_tx else {
            return;
        };

        if let Err(e) = close_recv_tx.send(DtlsClientClose) {
            warn!("close recv tx is closed before set to None: {e}");
        }

        self.close_recv_tx = None;
        self.recv_rx = None;   
    }
}
