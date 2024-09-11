use anyhow::Context;
use bevy::prelude::*;
use bytes::Bytes;
use super::dtls_server::{ConnIndex, DtlsServer, DtlsServerTimeout};

#[derive(Event, Debug)]
pub enum DtlsServerError {
    SendTimeout {
        conn_index: ConnIndex,
        bytes: Bytes
    },
    RecvTimeout {
        conn_index: ConnIndex
    },
    Fatal {
        err: anyhow::Error
    },
    ConnFatal {
        conn_index: ConnIndex,
        err: anyhow::Error
    }
}

pub fn timeout_event_system(
    mut dtls_server: ResMut<DtlsServer>,
    mut errors: EventWriter<DtlsServerError>
) {
    loop {
        let Err(e) = dtls_server.timeout_check() else {
            return;
        };
    
        match e {
            DtlsServerTimeout::Send { conn_index, bytes } => {
                errors.send(DtlsServerError::SendTimeout { 
                    conn_index, 
                    bytes 
                }); 
            }
            DtlsServerTimeout::Recv(idx) => {
                errors.send(DtlsServerError::RecvTimeout { 
                    conn_index: idx 
                });
            }
        }
    }
}

pub fn fatal_event_system(
    mut dtls_server: ResMut<DtlsServer>,
    mut errors: EventWriter<DtlsServerError>
) {
    let health = dtls_server.health_check();
    if let Some(r) = health.listener {
        if let Err(e) = r.context("fatal error from listener") {
            errors.send(DtlsServerError::Fatal { 
                err: e 
            });
        }
    }
    for (idx, r) in health.sender {
        if let Err(e) = r.context("fatal error from sender") {
            errors.send(DtlsServerError::ConnFatal { 
                conn_index: idx, 
                err: e 
            });
        }
    }
    for (idx, r) in health.recver {
        if let Err(e) = r.context("fatal error from recver") {
            errors.send(DtlsServerError::ConnFatal { 
                conn_index: idx, 
                err: e 
            });
        }
    }
}
