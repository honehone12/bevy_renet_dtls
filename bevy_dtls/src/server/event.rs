use anyhow::anyhow;
use bevy::prelude::*;
use bytes::Bytes;
use super::dtls_server::{ConnIndex, DtlsServer, DtlsServerTimeout};

#[derive(Event, Debug)]
pub enum DtlsServerEvent {
    SendTimeout {
        conn_index: ConnIndex,
        bytes: Bytes
    },
    RecvTimeout {
        conn_index: ConnIndex
    },
    Error {
        err: anyhow::Error
    },
    ConnError {
        conn_index: ConnIndex,
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
                    conn_index, 
                    bytes 
                }); 
            }
            DtlsServerTimeout::Recv(idx) => {
                errors.send(DtlsServerEvent::RecvTimeout { 
                    conn_index: idx 
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
                conn_index: idx, 
                err: anyhow!("fatal error from sender: {e}")
            });
        }
    }
    for (idx, r) in health.recver {
        if let Err(e) = r {
            errors.send(DtlsServerEvent::ConnError { 
                conn_index: idx, 
                err: anyhow!("fatal error from recver: {e}")
            });
        }
    }
}
