use std::{
    collections::HashMap, 
    net::IpAddr, 
    sync::{Arc, RwLock as StdRwLock}, 
    time::Duration
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

#[derive(Clone)]
pub struct DtlsServerConfig {
    pub listen_addr: IpAddr,
    pub listen_port: u16,
    pub cert_option: ServerCertOption
}

impl DtlsServerConfig {
    async fn listen(self)
    -> anyhow::Result<Arc<dyn Listener + Sync + Send>> {
        let listener = listener::listen(
            (self.listen_addr, self.listen_port), 
            self.cert_option.to_dtls_config()?
        )
        .await?;

        debug!("dtls server listening at {}", self.listen_addr);
        Ok(Arc::new(listener))
    }
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
pub struct DtlsConnHealth {
    pub conn_index: ConnIndex,
    pub sender: Option<anyhow::Result<()>>,
    pub recver: Option<anyhow::Result<()>>,
    pub closed: bool
}

#[derive(Debug)]
pub struct DtlsServerHealth {
    pub listener: Option<anyhow::Result<()>>,
    pub conns: Vec<DtlsConnHealth>
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

    async fn acpt_loop(mut self) -> anyhow::Result<()> {
        // start index from 1
        // because server wants reserve 0
        let mut index: u64 = 1;

        let result = loop {
            let (conn, addr) = select! {
                biased;

                Some(_) = self.close_rx.recv() => break Ok(()),
                r = self.listener.accept() => {
                    match r {
                        Ok(ca) => ca,
                        Err(e) => break Err(anyhow!(e)),
                    }
                }
                else => {
                    warn!(
                        "is dtls server dropped before disconnection? \
                        acpter loop is closing anyway"
                    );
                    break Ok(());
                }
            };

            if self.conn_map.read()
            .unwrap()
            .len() >= self.max_clients {
                warn!("{addr} is trying to connect, but exceeded max clients");
                if let Err(e) = conn.close().await {
                    error!("error on disconnect {addr}: {e}");
                }
                continue;
            }

            let idx = index;
            index = match index.checked_add(1) {
                Some(i) => i,
                None => {
                    if let Err(e) = conn.close().await {
                        error!("error on disconnect {addr}: {e}");
                    }
                    break Err(anyhow!("conn index overflow"));
                }
            };
            
            if let Err(e) = self.acpt_tx.send(ConnIndex(idx)) {
                if let Err(e) = conn.close().await {
                    error!("error on disconnect {addr}: {e}");
                }
                break Err(anyhow!(e));
            }

            let mut w = self.conn_map.write()
            .unwrap();
            debug_assert!(!w.contains_key(&idx));

            w.insert(idx, DtlsConn::new(conn));
            debug!("conn from {addr} accepted");
        };

        self.listener.close().await?;
        debug!("dtls server listener is closed");
        result
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

    async fn recv_loop(mut self) -> anyhow::Result<()> {
        let mut buf = BytesMut::zeroed(self.buf_size);
        let timeout_dur = self.timeout_secs();

        let result = loop {
            let (n, addr) = select! {
                biased;

                Some(_) = self.close_rx.recv() => break Ok(()),
                r = self.conn.recv_from(&mut buf) => {
                    match r {
                        Ok(na) => na,
                        Err(e) => break Err(anyhow!("conn {:?}: {e}", self.conn_idx))
                    }
                }
                () = sleep(timeout_dur) => {
                    if let Err(e) = self.timeout_tx.send(
                        DtlsServerTimeout::Recv(self.conn_idx)
                    ) {
                        break Err(anyhow!("conn {:?}: {e}", self.conn_idx));
                    }
                    continue;
                }
                else => {
                    warn!(
                        "is dtls conn {:?} closed before disconnection? \
                        recver loop is closing anyway", 
                        self.conn_idx
                    );
                    break Ok(());
                }
            };

            let recved = buf.split_to(n)
            .freeze();
            if let Err(e) = self.recv_tx.send((self.conn_idx, recved)) {
                break Err(anyhow!(e));
            }

            buf.resize(self.buf_size, 0);
            trace!("received {n}bytes from {:?}:{addr}", self.conn_idx);
        };

        self.conn.close().await?;
        debug!("dtls server recv loop: {:?} is closed", self.conn_idx);
        result
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

    async fn send_loop(mut self) -> anyhow::Result<()> {
        let result = loop {
            select! {
                biased;

                Some(_) = self.close_rx.recv() => break Ok(()),
                Some(msg) = self.send_rx.recv() => {
                    match timeout(
                        self.timeout_secs(),
                        self.conn.send(&msg)
                    )
                    .await {
                        Ok(r) => {
                            match r {
                                Ok(n) => trace!("sent {n} bytes to {:?}", self.conn_idx),
                                Err(e) => break Err(anyhow!("conn {:?}: {e}", self.conn_idx))
                            }
                        }
                        Err(_) => {
                            if let Err(e) = self.timeout_tx.send(DtlsServerTimeout::Send { 
                                conn_index: self.conn_idx, 
                                bytes: msg 
                            }) {
                                break Err(anyhow!("conn {:?}: {e}", self.conn_idx));
                            }
                        }
                    }
                }
                else => {
                    warn!(
                        "is dtls conn {:?} closed before disconnection? \
                        sender loop is closing anyway", 
                        self.conn_idx
                    );
                    break Ok(());
                }
            }
        };

        self.conn.close().await?;
        debug!("dtls server send loop {:?} is closed", self.conn_idx);
        result
    }
}

pub(super) struct DtlsConn {
    conn: Arc<dyn Conn + Sync + Send>,
    is_running: bool,

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
            is_running: false,
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
    pub fn is_closed(&self) -> bool {
        // set closed by conn healtch check
        self.conn_map.read()
        .unwrap()
        .is_empty()        
        
        // set closed by call ing close
        &&self.listener.is_none()
        && self.acpt_handle.is_none()
        && self.acpt_rx.is_none()
        && self.close_acpt_tx.is_none()
        && self.recv_tx.is_none()
        && self.recv_rx.is_none()
        && self.timeout_rx.is_none()
        && self.timeout_tx.is_none()
    }

    #[inline]
    pub fn connected_clients(&self) -> usize {
        let r = self.conn_map.read().unwrap();
        r.len()
    }

    #[inline]
    pub fn client_indices(&mut self) -> Vec<u64> {
        let ks = {
            self.conn_map.read()
            .unwrap()
            .keys()
            .cloned()
            .collect()
        };
        ks
    }

    #[inline]
    pub fn start(&mut self, config: DtlsServerConfig)
    -> anyhow::Result<()> {
        if !self.is_closed() {
            bail!("dtls server is not closed");
        }

        self.start_listen(config)?;
        self.start_acpt_loop()
    }

    #[inline]
    pub fn start_conn(&mut self, conn_index: ConnIndex)
    -> anyhow::Result<()> {
        self.start_recv_loop(conn_index)?;
        self.start_send_loop(conn_index)
    }

    #[inline]
    pub fn has_conn(&self, conn_idx: u64) -> bool {
        self.conn_map.read()
        .unwrap()
        .contains_key(&conn_idx)
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
            bail!(
                "conn {conn_index} is not started or is disconnected: \
                dtls conn is None"
            );
        };
        let Some(ref send_tx) = dtls_conn.send_tx else {
            bail!(
                "conn {conn_index} is not started or is disconnected: \
                send tx is None"
            );
        };

        if let Err(e) = send_tx.send(message) {
            bail!("conn {conn_index} is not started or is disconnected: {e}");
        }
        Ok(())
    }

    pub fn broadcast(&self, message: Bytes) -> anyhow::Result<()> {
        let r = self.conn_map.read()
        .unwrap();

        for (idx, ref dtls_conn) in r.iter() {
            let Some(ref send_tx) = dtls_conn.send_tx else {
                warn!("skipping {idx} that is not started or already closed");
                continue;
            };
    
            if let Err(e) = send_tx.send(message.clone()) {
                warn!(
                    "skipping {idx} with error: {e} \
                    that is not started or already closed"
                );
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
                    debug!("recver loop looks closed before disconnection: {e}");
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
                    error!(
                        "timeout tx is dropped or closed but rx is still living: {e} \
                        this is not observed generally"
                    );
                }
                Ok(())
            }
        }
    }

    #[inline]
    pub fn health_check(&mut self) -> DtlsServerHealth {
        DtlsServerHealth{
            listener: self.health_check_acpt(),
            conns: self.health_check_conn_loop()
        }
    }

    pub fn disconnect(&mut self, conn_index: u64) {
        let mut w = self.conn_map.write()
        .unwrap();
        if let Some(dtls_conn) = w.get_mut(&conn_index) {
            if let Some(ref close_recv_tx) = dtls_conn.close_recv_tx {
                if let Err(e) = close_recv_tx.send(DtlsServerClose) {
                    debug!("recver loop {conn_index} looks alredy closed: {e}");
                }
    
                dtls_conn.close_recv_tx = None;    
            };
    
            if let Some(ref close_send_tx) = dtls_conn.close_send_tx {
                if let Err(e) = close_send_tx.send(DtlsServerClose) {
                    debug!("sender loop {conn_index} looks already closed: {e}");
                }
    
                dtls_conn.close_send_tx = None;
            }
    
            dtls_conn.send_tx = None;    
        }
    }

    pub fn disconnect_all(&mut self) {
        let ks: Vec<u64> = {
            self.conn_map.read()
            .unwrap()
            .keys()
            .cloned()
            .collect()
        };
        
        for idx in ks {
            self.disconnect(idx);
        }
    }

    pub fn close(&mut self) {
        self.close_acpt_loop();

        self.recv_tx = None;
        self.recv_rx = None;
        self.timeout_tx = None;
        self.timeout_rx = None;
    }

    fn start_listen(&mut self, config: DtlsServerConfig) 
    -> anyhow::Result<()> {
        let listener = future::block_on(
            self.runtime.spawn(config.listen())
        )??;
        self.listener = Some(listener);

        Ok(())
    }

    fn start_acpt_loop(&mut self)
    -> anyhow::Result<()> {
        if self.acpt_handle.is_some() {
            bail!("join handle exists, or health_check is not called");
        }

        let (recv_tx, recv_rx) = tokio_channel::<(ConnIndex, Bytes)>();
        self.recv_tx = Some(recv_tx);
        self.recv_rx = Some(recv_rx);
        let (timeout_tx, timeout_rx) = tokio_channel::<DtlsServerTimeout>();
        self.timeout_tx = Some(timeout_tx);
        self.timeout_rx = Some(timeout_rx);

        let (
            acpt_rx,
            close_tx,
            acpter
        ) = DtlsServerAcpter::new(
            self.max_clients,
            match self.listener {
                Some(ref l) => Arc::clone(l),
                None => bail!("listener is None")
            }, 
            Arc::clone(&self.conn_map)
        );
        
        self.acpt_rx = Some(acpt_rx);
        self.close_acpt_tx = Some(close_tx);
        
        let handle = self.runtime.spawn(acpter.acpt_loop());
        self.acpt_handle = Some(handle);

        debug!("acpt loop is started");
        Ok(())
    }

    fn health_check_acpt(&mut self) 
    -> Option<anyhow::Result<()>> {
        let handle_ref = self.acpt_handle.as_ref()?;

        if !handle_ref.is_finished() {
            return None;
        }

        let handle = self.acpt_handle.take()?;
        self.listener = None;
        match future::block_on(handle) {
            Ok(r) => Some(r),
            Err(e) => Some(Err(anyhow!(e)))
        }
    }

    fn close_acpt_loop(&mut self) {
        if let Some(ref close_acpt_tx) = self.close_acpt_tx {
            if let Err(e) = close_acpt_tx.send(DtlsServerClose) {
                debug!("acpter loop looks already closed: {e}");
            }
        }

        self.close_acpt_tx = None;
        self.acpt_rx = None;
    }

    fn start_recv_loop(&self, conn_idx: ConnIndex) 
    -> anyhow::Result<()> {
        let mut w = self.conn_map.write()
        .unwrap();
        let Some(dtls_conn) = w.get_mut(&conn_idx.0) else {
            bail!("dtls conn {conn_idx:?} is None");
        };

        if dtls_conn.recv_handle.is_some() {
            bail!("join handle already exists, or health_check is not called");
        }

        let (close_tx, recver) = DtlsServerRecver::new(
            conn_idx, 
            Arc::clone(&dtls_conn.conn), 
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

        let handle = self.runtime.spawn(recver.recv_loop());
        dtls_conn.recv_handle = Some(handle);
        dtls_conn.is_running = true;
        
        debug!("recv loop {conn_idx:?} has started");
        Ok(())
    }

    fn start_send_loop(&mut self, conn_idx: ConnIndex) 
    -> anyhow::Result<()> {
        let mut w = self.conn_map.write()
        .unwrap();
        let Some(dtls_conn) = w.get_mut(&conn_idx.0) else {
            bail!("dtls conn: {conn_idx:?} is None");
        };

        if dtls_conn.send_handle.is_some() {
            bail!("join handle already exists, or health_check is not called");
        }

        let (send_tx, close_tx, sender) = DtlsServerSender::new(
            conn_idx, 
            Arc::clone(&dtls_conn.conn), 
            self.send_timeout_secs,
            match self.timeout_tx {
                Some(ref tx) => tx.clone(),
                None => bail!("timeout tx is still None")
            }
        );

        dtls_conn.send_tx = Some(send_tx);
        dtls_conn.close_send_tx = Some(close_tx);

        let handle = self.runtime.spawn(sender.send_loop());
        dtls_conn.send_handle = Some(handle);
        dtls_conn.is_running = true;

        debug!("send loop {conn_idx:?} has started");
        Ok(())
    }

    fn health_check_conn_loop(&mut self)
    -> Vec<DtlsConnHealth> {
        let mut conns_health = vec![];
            
        let conn_statuses = {
            let mut s = vec![];
            let r = self.conn_map.read()
            .unwrap();
            for (idx, dtls_conn) in r.iter() {
                let sender_finished = if let Some(ref handle_ref) = dtls_conn.send_handle {
                    handle_ref.is_finished()
                } else {
                    false
                };

                let recver_finished = if let Some(ref handle_ref) = dtls_conn.recv_handle {
                    handle_ref.is_finished()
                } else {
                    false
                }; 

                if sender_finished || recver_finished {
                    s.push((*idx, sender_finished, recver_finished));
                }
            }
            s
        };

        let mut w = self.conn_map.write()
        .unwrap();
        for (idx, sender_finished, recver_finished) in conn_statuses {
            let dtls_conn = w.get_mut(&idx)
            .unwrap();

            let sender_health = if sender_finished {
                let handle = dtls_conn.send_handle
                .take()
                .unwrap();
                let r = match future::block_on(handle) {
                    Ok(r) => r,
                    Err(e) => Err(anyhow!(e))
                };
                Some(r)
            } else {
                None
            };

            let recver_health = if recver_finished {
                let handle = dtls_conn.recv_handle
                .take()
                .unwrap();
                let r = match future::block_on(handle) {
                    Ok(r) => r,
                    Err(e) => Err(anyhow!(e))
                };
                Some(r)
            } else {
                None
            };

            let closed = dtls_conn.is_running
            && dtls_conn.send_handle.is_none()
            && dtls_conn.recv_handle.is_none();
        
            if closed {
                w.remove(&idx);
            }

            conns_health.push(DtlsConnHealth{
                conn_index: ConnIndex(idx),
                sender: sender_health,
                recver: recver_health,
                closed
            });
        }
        conns_health
    }
}
