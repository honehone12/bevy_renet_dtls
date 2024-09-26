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
    }
}

pub fn timeout_event_system(
    mut dtls_server: ResMut<DtlsServer>,
    mut errors: EventWriter<DtlsServerEvent>
) {
    loop {
        let Err(e) = dtls_server.timeout_check() else {
            return;
        };
    
        match e {
            DtlsServerTimeout::Send { conn_index, bytes } => {
                errors.send(DtlsServerEvent::SendTimeout { 
                    conn_index: conn_index.index(), 
                    bytes 
                }); 
            }
            DtlsServerTimeout::Recv(idx) => {
                errors.send(DtlsServerEvent::RecvTimeout { 
                    conn_index: idx.index() 
                });
            }
        }
    }
}

pub fn health_event_system(
    mut dtls_server: ResMut<DtlsServer>,
    mut errors: EventWriter<DtlsServerEvent>
) {
    let health = dtls_server.health_check();
    if let Some(r) = health.listener {
        if let Err(e) = r {
            errors.send(DtlsServerEvent::Error { 
                err: anyhow!("fatal error from listener: {e}")
            });
        }
    }
    for (idx, r) in health.sender {
        if let Err(e) = r {
            errors.send(DtlsServerEvent::ConnError { 
                conn_index: idx.index(), 
                err: anyhow!("fatal error from sender: {e}")
            });
        }
    }
    for (idx, r) in health.recver {
        if let Err(e) = r {
            errors.send(DtlsServerEvent::ConnError { 
                conn_index: idx.index(), 
                err: anyhow!("fatal error from recver: {e}")
            });
        }
    }
}
