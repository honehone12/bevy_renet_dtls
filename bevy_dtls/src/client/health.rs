use anyhow::anyhow;
use bevy::prelude::*;
use bytes::Bytes;
use super::dtls_client::{DtlsClient, DtlsClientTimeout};

#[derive(Event, Debug)]
pub enum DtlsClientError {
    SendTimeout {
        bytes: Bytes
    },
    Fatal {
        err: anyhow::Error
    }
}

pub fn timeout_event_system(
    mut dtls_client: ResMut<DtlsClient>,
    mut errors: EventWriter<DtlsClientError>
) {
    loop {
        let Err(e) = dtls_client.timeout_check() else {
            return;
        };

        match e {
            DtlsClientTimeout::Send(bytes) => {
                errors.send(DtlsClientError::SendTimeout { 
                    bytes
                });
            }
        }
    }
}

pub fn fatal_event_system(
    mut dtls_client: ResMut<DtlsClient>,
    mut errors: EventWriter<DtlsClientError>
) {
    let health = dtls_client.health_check();
    if let Some(r) = health.sender {
        if let Err(e) = r {
            errors.send(DtlsClientError::Fatal { 
                err: anyhow!("fatal error from sender: {e}")
            });
        }
    }
    if let Some(r) = health.recver {
        if let Err(e) = r {
            errors.send(DtlsClientError::Fatal { 
                err: anyhow!("fatal error from recver: {e}")
            });
        }
    }
}
