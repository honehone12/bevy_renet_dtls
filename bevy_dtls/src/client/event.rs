use anyhow::anyhow;
use bevy::prelude::*;
use bytes::Bytes;
use super::dtls_client::{DtlsClient, DtlsClientTimeout};

#[derive(Event, Debug)]
pub enum DtlsClientEvent {
    SendTimeout {
        bytes: Bytes
    },
    Error {
        err: anyhow::Error
    },
    ConnClosed
}

pub fn timeout_event_system(
    mut dtls_client: ResMut<DtlsClient>,
    mut dtls_events: EventWriter<DtlsClientEvent>
) {
    loop {
        let Err(e) = dtls_client.timeout_check() else {
            return;
        };

        match e {
            DtlsClientTimeout::Send(bytes) => {
                dtls_events.send(DtlsClientEvent::SendTimeout { 
                    bytes
                });
            }
        }
    }
}

pub fn health_event_system(
    mut dtls_client: ResMut<DtlsClient>,
    mut dtls_events: EventWriter<DtlsClientEvent>
) {
    let health = dtls_client.health_check();
    if let Some(Err(e)) = health.sender {
        dtls_events.send(DtlsClientEvent::Error { 
            err: anyhow!("error from sender: {e}")
        });
    }
    if let Some(Err(e)) = health.recver {
        dtls_events.send(DtlsClientEvent::Error { 
            err: anyhow!("error from recver: {e}")
        });
    }
    if health.closed {
        dtls_events.send(DtlsClientEvent::ConnClosed);
    }
}
