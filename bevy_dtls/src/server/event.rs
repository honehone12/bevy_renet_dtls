use anyhow::anyhow;
use bevy::prelude::*;
use bytes::Bytes;
use super::dtls_server::{DtlsServer, DtlsServerTimeout};

#[derive(Event, Debug)]
pub enum DtlsServerEvent {
    SendTimeout {
        conn_index: u64,
        bytes: Bytes
    },
    RecvTimeout {
        conn_index: u64
    },
    Error {
        err: anyhow::Error
    },
    ConnError {
        conn_index: u64,
        err: anyhow::Error
    },
    ConnClosed {
        conn_index: u64
    },
    ListenerClosed
}

pub fn timeout_event_system(
    mut dtls_server: ResMut<DtlsServer>,
    mut dtls_events: EventWriter<DtlsServerEvent>
) {
    loop {
        let Err(e) = dtls_server.timeout_check() else {
            return;
        };
    
        match e {
            DtlsServerTimeout::Send { conn_index, bytes } => {
                dtls_events.send(DtlsServerEvent::SendTimeout { 
                    conn_index: conn_index.index(), 
                    bytes 
                }); 
            }
            DtlsServerTimeout::Recv(idx) => {
                dtls_events.send(DtlsServerEvent::RecvTimeout { 
                    conn_index: idx.index() 
                });
            }
        }
    }
}

pub fn health_event_system(
    mut dtls_server: ResMut<DtlsServer>,
    mut dtls_events: EventWriter<DtlsServerEvent>
) {
    let health = dtls_server.health_check();
    if let Some(r) = health.listener {
        if let Err(e) = r {
            dtls_events.send(DtlsServerEvent::Error { 
                err: anyhow!("error from listener: {e}")
            });
        }

        dtls_events.send(DtlsServerEvent::ListenerClosed);
    }

    for conn_health in health.conns {
        if let Some(Err(e)) = conn_health.sender {
            dtls_events.send(DtlsServerEvent::ConnError { 
                conn_index: conn_health.conn_index.index(), 
                err: anyhow!("error from sender: {e}")
            });
        }
        if let Some(Err(e)) = conn_health.recver {
            dtls_events.send(DtlsServerEvent::ConnError { 
                conn_index: conn_health.conn_index.index(), 
                err: anyhow!("error from recver: {e}")
            });
        }
        if conn_health.closed {
            dtls_events.send(DtlsServerEvent::ConnClosed { 
                conn_index: conn_health.conn_index.index() 
            });
        }
    }
}
