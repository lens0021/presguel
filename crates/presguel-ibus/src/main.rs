//! presguel-ibus — 날개셋 호환 ibus 한글 입력기 (zbus 프런트엔드).
//!
//! ibus-daemon 의 사설 D-Bus 에 붙어 Factory 를 등록하고, 데몬이 요청하면 엔진
//! 객체를 만들어 한글을 조합한다. 참고: `research/03-ibus-zbus.md`.

mod addr;
mod engine;
mod factory;
mod ibus_property;
mod ibus_text;
mod settings;

use std::path::PathBuf;

use presguel_core::Config;
use zbus::connection::Builder;

use crate::factory::Factory;

/// 데몬에 요청할 well-known 버스 이름(= 컴포넌트 xml 의 <name>).
const BUS_NAME: &str = "org.freedesktop.IBus.Presguel";
const FACTORY_PATH: &str = "/org/freedesktop/IBus/Factory";

/// 설정 파일 경로: `$PRESGUEL_CONFIG` → `~/.config/presguel/nalgaeset.xml`.
fn resolve_config_path() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("PRESGUEL_CONFIG") {
        if !p.is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".config")))
        .ok_or_else(|| "HOME/XDG_CONFIG_HOME 가 없음".to_string())?;
    Ok(base.join("presguel").join("nalgaeset.xml"))
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 설정 로드 + 컴파일.
    let path = resolve_config_path()?;
    let xml = std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "설정 {} 읽기 실패: {e}\n\
             PRESGUEL_CONFIG 환경변수로 nalgaeset.xml 경로를 지정하거나 \
             ~/.config/presguel/nalgaeset.xml 에 두세요.",
            path.display()
        )
    })?;
    let cfg = std::sync::Arc::new(Config::parse(&xml)?);
    eprintln!(
        "presguel-ibus: 설정 {} ({} 항목, 기본 {}) 로드",
        path.display(),
        cfg.entries.len(),
        cfg.default_entry
    );

    // IBus 사설 버스에 연결.
    let address = addr::find_ibus_address()?;
    let conn = Builder::address(address.as_str())?.build().await?;
    let _ = conn.unique_name(); // Hello 핸드셰이크 완료 확인

    // 팩토리 등록 + 이름 요청.
    let factory = Factory::new(conn.clone(), cfg);
    conn.object_server().at(FACTORY_PATH, factory).await?;
    conn.request_name(BUS_NAME).await?;
    eprintln!("presguel-ibus: {BUS_NAME} 등록 완료, 대기 중");

    // Ctrl-C 또는 종료까지 대기.
    tokio::signal::ctrl_c().await.ok();
    Ok(())
}
