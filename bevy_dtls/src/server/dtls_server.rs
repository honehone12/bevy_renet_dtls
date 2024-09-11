use std::{
    collections::HashMap, net::IpAddr, sync::{Arc, RwLock as StdRwLock}, time::Duration
};
use anyhow::{anyhow, bail};
use bevy::{
    prelude::*, 
    tasks::futures_lite::future, 
};
use tokio::{
    runtime::{self, Runtime}, 
    select, 
    sync::mpsc::{
        error::TryRecvError, 
        unbounded_channel as tokio_channel, 
        UnboundedReceiver as TokioRx, 
        UnboundedSender as TokioTx
    }, 
    task::JoinHandle,
    time::{timeout, sleep}
};
use webrtc_dtls::listener;
use webrtc_util::conn::{Listener, Conn};
use bytes::{Bytes, BytesMut};
use super::cert_option::ServerCertOption;

#[derive(Clone, Copy, Debug)]
pub struct ConnIndex(u64);

impl ConnIndex {
    #[inline]
    pub fn index(&self) -> u64 {
        self.0
    }
}

pub struct DtlsServerConfig {
    pub listen_addr: IpAddr,
    pub listen_port: u16,
    pub cert_option: ServerCertOption
}

#[derive(Debug)]
pub enum DtlsServerTimeout {
    Send{
        conn_index: ConnIndex,
        bytes: Bytes
    },
    Recv(ConnIndex)
}

struct DtlsServerClose;

#[derive(Debug)]
pub struct DtlsServerHealth {
    pub listener: Option<anyhow::Result<()>>,
    pub sender: Vec<(ConnIndex, anyhow::Result<()>)>,
    pub recver: Vec<(ConnIndex, anyhow::Result<()>)>
}

struct DtlsServerAcpter {
    max_clients: usize,
    listener: Arc<dyn Listener + Sync + Send>,
    conn_map: Arc<StdRwLock<HashMap<u64, DtlsConn>>>,
    acpt_tx:  TokioTx<ConnIndex>,
    close_rx: TokioRx<DtlsServerClose>
}

impl DtlsServerAcpter {
    #[inline]
    fn new(
        max_clients: usize,
        listener: Arc<dyn Listener + Sync + Send>,
        conn_map: Arc<StdRwLock<HashMap<u64, DtlsConn>>>
    ) -> (TokioRx<ConnIndex>, TokioTx<DtlsServerClose>, Self) {
        let (acpt_tx, acpt_rx) = tokio_channel::<ConnIndex>();
        let (close_tx, close_rx) = tokio_channel::<DtlsServerClose>();

        (acpt_rx, close_tx, Self{
            max_clients,
            listener,
            conn_map,
            acpt_tx,
            close_rx,
        })
    }
}

struct DtlsServerRecver {
    conn_idx: ConnIndex,
    conn: Arc<dyn Conn + Sync + Send>,
    buf_size: usize,
    timeout_secs: Option<u64>,

    recv_tx: TokioTx<(ConnIndex, Bytes)>,
    timeout_tx: TokioTx<DtlsServerTimeout>,
    close_rx: TokioRx<DtlsServerClose>
}

impl DtlsServerRecver {
    #[inline]
    fn new(
        conn_idx: ConnIndex,
        conn: Arc<dyn Conn + Sync + Send>,
        buf_size: usize,
        timeout_secs: Option<u64>,
        recv_tx: TokioTx<(ConnIndex, Bytes)>,
        timeout_tx: TokioTx<DtlsServerTimeout>
    ) -> (TokioTx<DtlsServerClose>, Self) {
        let (close_tx, close_rx) = tokio_channel::<DtlsServerClose>();

        (close_tx, Self{
            conn_idx,
            conn,
            buf_size,
            timeout_secs,
            recv_tx,
            timeout_tx,
            close_rx,
        })
    }

    #[inline]
    fn timeout_secs(&self) -> Duration {
        match self.timeout_secs {
            Some(t) => Duration::from_secs(t),
            None => Duration::MAX
        }
    }
}

struct DtlsServerSender {
    conn_idx: ConnIndex,
    conn: Arc<dyn Conn + Sync + Send>,
    timeout_secs: u64,

    send_rx: TokioRx<Bytes>,
    timeout_tx: TokioTx<DtlsServerTimeout>,
    close_rx: TokioRx<DtlsServerClose>
}

impl DtlsServerSender {
    #[inline]
    fn new(
        conn_idx: ConnIndex, 
        conn: Arc<dyn Conn + Sync + Send>,
        timeout_secs: u64,
        timeout_tx: TokioTx<DtlsServerTimeout>
    ) -> (TokioTx<Bytes>, TokioTx<DtlsServerClose>, Self) {
        let (send_tx, send_rx) = tokio_channel::<Bytes>();
        let (close_tx, close_rx) = tokio_channel::<DtlsServerClose>();
 
        (send_tx, close_tx, Self{
            conn_idx,
            conn,
            timeout_secs,
            send_rx,
            timeout_tx,
            close_rx
        })
    }

    #[inline]
    fn timeout_secs(&self) -> Duration {
        Duration::from_secs(self.timeout_secs)
    }
}

pub(super) struct DtlsConn {
    conn: Arc<dyn Conn + Sync + Send>,
    
    recv_handle: Option<JoinHandle<anyhow::Result<()>>>,
    close_recv_tx: Option<TokioTx<DtlsServerClose>>,

    send_handle: Option<JoinHandle<anyhow::Result<()>>>,
    send_tx: Option<TokioTx<Bytes>>,
    close_send_tx: Option<TokioTx<DtlsServerClose>>
}

impl DtlsConn {
    #[inline]
    pub(super) fn new(conn: Arc<dyn Conn + Sync + Send>) -> Self {
        Self{
            conn,
            recv_handle: None,
            close_recv_tx: None,
            send_handle: None,
            send_tx: None,
            close_send_tx: None,
        }
    }
}

#[derive(Resource)]
pub struct DtlsServer {
    runtime: Arc<Runtime>,
    
    max_clients: usize,
    listener: Option<Arc<dyn Listener + Sync + Send>>,
    acpt_handle: Option<JoinHandle<anyhow::Result<()>>>,
    acpt_rx: Option<TokioRx<ConnIndex>>,
    close_acpt_tx: Option<TokioTx<DtlsServerClose>>,
    
    conn_map: Arc<StdRwLock<HashMap<u64, DtlsConn>>>,

    send_timeout_secs: u64,

    recv_buf_size: usize,
    recv_timeout_secs: Option<u64>,
    recv_tx: Option<TokioTx<(ConnIndex, Bytes)>>,
    recv_rx: Option<TokioRx<(ConnIndex, Bytes)>>,

    timeout_tx: Option<TokioTx<DtlsServerTimeout>>,
    timeout_rx: Option<TokioRx<DtlsServerTimeout>>
}

impl DtlsServer {
    #[inline]
    pub fn new(
        max_clients: usize,
        recv_buf_size: usize, 
        send_timeout_secs: u64,
        recv_timeout_secs: Option<u64>
    ) -> anyhow::Result<Self> {
        let rt = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

        Ok(Self { 
            runtime: Arc::new(rt),

            max_clients,
            listener: None, 
            acpt_handle: None,
            acpt_rx: None,
            close_acpt_tx: None,
            
            conn_map: default(),

            send_timeout_secs,

            recv_timeout_secs,
            recv_buf_size,
            recv_tx: None,
            recv_rx: None,

            timeout_rx: None,
            timeout_tx: None
        })
    }

    #[inline]
    pub fn clients_len(&self) -> usize {
        let r = self.conn_map.read().unwrap();
        r.len()
    }

    #[inline]
    pub fn start(&mut self, config: DtlsServerConfig)
    -> anyhow::Result<()> {
        self.start_listen(config)?;
        self.start_acpt_loop()
    }

    #[inline]
    pub fn start_conn(&mut self, conn_index: ConnIndex)
    -> anyhow::Result<()> {
        self.start_recv_loop(conn_index)?;
        self.start_send_loop(conn_index)
    }

    pub fn acpt(&mut self) -> Option<ConnIndex> {
        let Some(ref mut acpt_rx) = self.acpt_rx else {
            return None;
        };

        match acpt_rx.try_recv() {
            Ok(a) => Some(a),
            Err(TryRecvError::Empty) => None,
            Err(e) => {
                error!("acpt rx is closed before set to None: {e}");
                None
            }
        }
    }

    pub fn send(&self, conn_index: u64, message: Bytes) 
    -> anyhow::Result<()> {
        let r = self.conn_map.read()
        .unwrap();
        let Some(ref dtls_conn) = r.get(&conn_index) else {
            bail!("dtls conn: {conn_index} is None");
        };
        let Some(ref send_tx) = dtls_conn.send_tx else {
            bail!("send tx: {conn_index} is None");
        };

        if let Err(e) = send_tx.send(message) {
            bail!("conn: {conn_index} is not started or disconnected: {e}");
        }
        Ok(())
    }

    pub fn broadcast(&self, message: Bytes) -> anyhow::Result<()> {
        let r = self.conn_map.read()
        .unwrap();

        for (idx, ref dtls_conn) in r.iter() {
            let Some(ref send_tx) = dtls_conn.send_tx else {
                warn!("send tx: {idx} is None");
                continue;
            };
    
            if let Err(e) = send_tx.send(message.clone()) {
                warn!("conn: {idx} is not started or disconnected: {e}");
                continue;
            }
        }

        Ok(())
    }

    pub fn recv(&mut self) -> Option<(ConnIndex, Bytes)> {
        let Some(ref mut recv_rx) = self.recv_rx else {
            return None;
        };

        match recv_rx.try_recv() {
            Ok(ib) => Some(ib),
            Err(e) => {
                if matches!(e, TryRecvError::Disconnected) {
                    warn!("recv rx is closed before set to None: {e}");
                }
                None
            }
        }
    }

    pub fn timeout_check(&mut self)
    -> std::result::Result<(), DtlsServerTimeout> {
        let Some(ref mut timeout_rx) = self.timeout_rx else {
            return Ok(())
        };

        match timeout_rx.try_recv() {
            Ok(t) => Err(t),
            Err(e) => {
                if matches!(e, TryRecvError::Disconnected) {
                    warn!("timeout rx is closed before set to None: {e}");
                }
                Ok(())
            }
        }
    }

    #[inline]
    pub fn health_check(&mut self) -> DtlsServerHealth {
        DtlsServerHealth{
            listener: self.health_check_acpt(),
            sender: self.health_check_send(),
            recver: self.health_check_recv()
        }
    }

    pub fn close_conn(&mut self, conn_index: u64) {
        let mut w = self.conn_map.write()
        .unwrap();
        let Some(dtls_conn) = w.remove(&conn_index) else {
            return;
        };
        
        if let Some(close_recv_tx) = dtls_conn.close_recv_tx {
            if let Err(e) = close_recv_tx.send(DtlsServerClose) {
                error!("close recv tx: {conn_index} is closed before set to None: {e}");
            }    
        };

        if let Some(close_send_tx) = dtls_conn.close_send_tx {
            if let Err(e) = close_send_tx.send(DtlsServerClose) {
                error!("close recv tx: {conn_index} is closed before set to None: {e}");
            }
        }
    }

    pub fn close_all(&mut self) {
        let ks: Vec<u64> = {
            self.conn_map.read()
            .unwrap()
            .keys()
            .cloned()
            .collect()
        };

        for k in ks {
            self.close_conn(k);
        }

        self.close_acpt_loop();
        self.recv_tx = None;
        self.recv_rx = None;
        self.timeout_tx = None;
        self.timeout_rx = None;
    }

    fn start_listen(&mut self, config: DtlsServerConfig) 
    -> anyhow::Result<()> {
        let listener = future::block_on(
            self.runtime.spawn(Self::listen(config))
        )??;
        self.listener = Some(listener);

        let (recv_tx, recv_rx) = tokio_channel::<(ConnIndex, Bytes)>();
        self.recv_tx = Some(recv_tx);
        self.recv_rx = Some(recv_rx);
        let (timeout_tx, timeout_rx) = tokio_channel::<DtlsServerTimeout>();
        self.timeout_tx = Some(timeout_tx);
        self.timeout_rx = Some(timeout_rx);

        Ok(())
    }

    async fn listen(config: DtlsServerConfig)
    -> anyhow::Result<Arc<dyn Listener + Sync + Send>> {
        let listener = listener::listen(
            (config.listen_addr, config.listen_port), 
            config.cert_option.to_dtls_config()?
        ).await?;

        debug!("dtls server listening at {}", config.listen_addr);
        Ok(Arc::new(listener))
    }

    fn start_acpt_loop(&mut self)
    -> anyhow::Result<()> {
        if self.acpt_handle.is_some() {
            bail!("join handle exists, or health_check is not called");
        }

        let (
            acpt_rx,
            close_tx,
            acpter
        ) = DtlsServerAcpter::new(
            self.max_clients,
            match self.listener {
                Some(ref l) => l.clone(),
                None => bail!("listener is None")
            }, 
            self.conn_map.clone()
        );
        
        self.acpt_rx = Some(acpt_rx);
        self.close_acpt_tx = Some(close_tx);
        
        let handle = self.runtime.spawn(Self::acpt_loop(acpter));
        self.acpt_handle = Some(handle);

        debug!("acpt loop is started");
        Ok(())
    }

    async fn acpt_loop(mut acpter: DtlsServerAcpter) -> anyhow::Result<()> {
        let mut index = 0;

        let result = loop {
            let (conn, addr) = select! {
                biased;

                r = acpter.listener.accept() => {
                    match r {
                        Ok(ca) => ca,
                        Err(e) => break Err(anyhow!(e)),
                    }
                }
                Some(_) = acpter.close_rx.recv() => break Ok(()),
                else => {
                    error!("close acpt tx is dropped before rx is closed");
                    break Ok(());
                }
            };

            if acpter.conn_map.read()
            .unwrap()
            .len() >= acpter.max_clients {
                warn!("{addr} is trying to connect, but exceeded max clients");
                if let Err(e) = conn.close().await {
                    error!("error on disconnect {addr}: {e}");
                }
                continue;
            }

            let idx = index;
            index += 1;
            let mut w = acpter.conn_map.write()
            .unwrap();
            debug_assert!(!w.contains_key(&idx));
            w.insert(idx, DtlsConn::new(conn));

            if let Err(e) = acpter.acpt_tx.send(ConnIndex(idx)) {
                break Err(anyhow!(e));
            }
            debug!("conn from {addr} accepted");
        };

        acpter.listener.close().await?;
        debug!("dtls server listener is closed");
        result
    }

    fn health_check_acpt(&mut self) 
    -> Option<anyhow::Result<()>> {
        let handle_ref = self.acpt_handle.as_ref()?;

        if !handle_ref.is_finished() {
            return None;
        }

        let handle = self.acpt_handle.take()?;
        match future::block_on(handle) {
            Ok(r) => Some(r),
            Err(e) => Some(Err(anyhow!(e)))
        }
    }

    fn close_acpt_loop(&mut self) {
        let Some(ref close_acpt_tx) = self.close_acpt_tx else {
            return;
        };

        if let Err(e) = close_acpt_tx.send(DtlsServerClose) {
            error!("close listener tx is closed before set to None: {e}");
        }

        self.close_acpt_tx = None;
        self.acpt_rx = None;
        self.listener = None;
    }

    fn start_recv_loop(&self, conn_idx: ConnIndex) 
    -> anyhow::Result<()> {
        let mut w = self.conn_map.write()
        .unwrap();
        let Some(dtls_conn) = w.get_mut(&conn_idx.0) else {
            bail!("dtls conn: {} is None", conn_idx.0);
        };

        if dtls_conn.recv_handle.is_some() {
            bail!("join handle already exists, or health_check is not called");
        }

        let (close_tx, recver) = DtlsServerRecver::new(
            conn_idx, 
            dtls_conn.conn.clone(), 
            self.recv_buf_size, 
            self.recv_timeout_secs, 
            match self.recv_tx {
                Some(ref tx) => tx.clone(),
                None => bail!("recv tx is still None")
            },
            match self.timeout_tx {
                Some(ref tx) => tx.clone(),
                None => bail!("timeout tx is still None")
            }
        );

        dtls_conn.close_recv_tx = Some(close_tx);

        let handle = self.runtime.spawn(Self::recv_loop(recver));
        dtls_conn.recv_handle = Some(handle);
        
        debug!("recv loop: {} has started", conn_idx.0);
        Ok(())
    }

    async fn recv_loop(mut recver: DtlsServerRecver) -> anyhow::Result<()> {
        let mut buf = BytesMut::zeroed(recver.buf_size);
        let timeout_dur = recver.timeout_secs();

        let result = loop {
            let (n, addr) = select! {
                biased;

                r = recver.conn.recv_from(&mut buf) => {
                    match r {
                        Ok(na) => na,
                        Err(e) => break Err(anyhow!(e))
                    }
                }
                Some(_) = recver.close_rx.recv() => break Ok(()),
                () = sleep(timeout_dur) => {
                    if let Err(e) = recver.timeout_tx.send(
                        DtlsServerTimeout::Recv(recver.conn_idx)
                    ) {
                        break Err(anyhow!(e));
                    }
                    continue;
                }
                else => {
                    error!(
                        "close recv tx: {} is closed before rx is closed", 
                        recver.conn_idx.0
                    );
                    break Ok(());
                }
            };

            let recved = buf.split_to(n)
            .freeze();
            if let Err(e) = recver.recv_tx.send((recver.conn_idx, recved)) {
                break Err(anyhow!(e));
            }

            buf.resize(recver.buf_size, 0);
            debug!("received {n}bytes from {}:{addr}", recver.conn_idx.0);
        };

        recver.conn.close().await?;
        debug!("dtls server recv loop: {} is closed", recver.conn_idx.0);
        result
    }

    fn health_check_recv(&mut self)
    -> Vec<(ConnIndex, anyhow::Result<()>)> {
        let finished = {
            let mut v = vec![];
            let mut w = self.conn_map.write()
            .unwrap();
            for (idx, dtls_conn) in w.iter_mut() {
                let Some(ref handle_ref) = dtls_conn.recv_handle else {
                    continue;
                };
                
                if !handle_ref.is_finished() {
                    continue;
                }

                let handle = dtls_conn.recv_handle.take()
                .unwrap();
                v.push((*idx, handle));
            }
            v
        };

        let mut results = vec![];
        for (idx, handle) in finished {
            let r = match future::block_on(handle) {
                Ok(r) => r,
                Err(e) => Err(anyhow!(e))
            };
            results.push((ConnIndex(idx), r));
        }
        results
    }

    fn start_send_loop(&mut self, conn_idx: ConnIndex) 
    -> anyhow::Result<()> {
        let mut w = self.conn_map.write()
        .unwrap();
        let Some(dtls_conn) = w.get_mut(&conn_idx.0) else {
            bail!("dtls conn: {} is None", conn_idx.0);
        };

        if dtls_conn.send_handle.is_some() {
            bail!("join handle already exists, or health_check is not called");
        }

        let (send_tx, close_tx, sender) = DtlsServerSender::new(
            conn_idx, 
            dtls_conn.conn.clone(), 
            self.send_timeout_secs,
            match self.timeout_tx {
                Some(ref tx) => tx.clone(),
                None => bail!("timeout tx is still None")
            }
        );

        dtls_conn.send_tx = Some(send_tx);
        dtls_conn.close_send_tx = Some(close_tx);

        let handle = self.runtime.spawn(Self::send_loop(sender));
        dtls_conn.send_handle = Some(handle);

        debug!("send loop: {} has started", conn_idx.0);
        Ok(())
    }

    async fn send_loop(mut sender: DtlsServerSender) -> anyhow::Result<()> {
        let result = loop {
            select! {
                biased;

                Some(msg) = sender.send_rx.recv() => {
                    match timeout(
                        sender.timeout_secs(),
                        sender.conn.send(&msg)
                    ).await {
                        Ok(r) => {
                            match r {
                                Ok(_) => (),
                                Err(e) => break Err(anyhow!(e))
                            }
                        }
                        Err(_) => {
                            if let Err(e) = sender.timeout_tx.send(DtlsServerTimeout::Send { 
                                conn_index: sender.conn_idx, 
                                bytes: msg 
                            }) {
                                break Err(anyhow!(e));
                            }
                        }
                    }
                }
                Some(_) = sender.close_rx.recv() => break Ok(()),
                else => {
                    warn!(
                        "close send tx: {} is closed before rx is closed", 
                        sender.conn_idx.0
                    );
                    break Ok(());
                }
            }
        };

        sender.conn.close().await?;
        debug!("dtls server send loop: {} is closed", sender.conn_idx.0);
        result
    }

    fn health_check_send(&mut self)
    -> Vec<(ConnIndex, anyhow::Result<()>)> {
        let finished = {
            let mut v = vec![];
            let mut w = self.conn_map.write()
            .unwrap();
            for (idx, dtls_conn) in w.iter_mut() {
                let Some(ref handle_ref) = dtls_conn.send_handle else {
                    continue;
                };
                
                if !handle_ref.is_finished() {
                    continue;
                }

                let handle = dtls_conn.send_handle.take()
                .unwrap();
                v.push((*idx, handle));
            }
            v
        };

        let mut results = vec![];
        for (idx, handle) in finished {
            let r = match future::block_on(handle) {
                Ok(r) => r,
                Err(e) => Err(anyhow!(e))
            };
            results.push((ConnIndex(idx), r));
        }
        results
    }
}
