//! `org.freedesktop.IBus.Factory` 구현. 데몬의 `CreateEngine` 요청마다 엔진 객체를
//! 만들어 객체 서버에 등록하고 그 경로를 돌려준다.

use std::sync::Arc;

use presguel_core::Config;
use zbus::zvariant::OwnedObjectPath;
use zbus::{fdo, interface, Connection};

use crate::engine::IBusEngine;

/// 엔진을 찍어내는 팩토리.
pub struct Factory {
    conn: Connection,
    config: Arc<Config>,
    next: u32,
}

impl Factory {
    pub fn new(conn: Connection, config: Arc<Config>) -> Self {
        Self { conn, config, next: 0 }
    }
}

#[interface(name = "org.freedesktop.IBus.Factory")]
impl Factory {
    async fn create_engine(&mut self, name: String) -> fdo::Result<OwnedObjectPath> {
        if name != "presguel" {
            return Err(fdo::Error::Failed(format!("알 수 없는 엔진: {name}")));
        }
        self.next += 1;
        let path = format!("/org/freedesktop/IBus/Engine/{}", self.next);
        let engine = IBusEngine::new(&self.config);
        self.conn
            .object_server()
            .at(path.as_str(), engine)
            .await
            .map_err(|e| fdo::Error::Failed(format!("엔진 등록 실패: {e}")))?;
        OwnedObjectPath::try_from(path).map_err(|e| fdo::Error::Failed(e.to_string()))
    }
}
