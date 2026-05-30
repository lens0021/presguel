//! presguel-ibus 엔진을 D-Bus 로 직접 구동해 보는 종단간 점검 도구.
//!
//! 실행 중인 presguel-ibus 프로세스(같은 ibus 버스에 등록됨)의 Factory 를 호출해
//! 엔진을 만들고, 키 시퀀스를 ProcessKeyEvent 로 보내며 CommitText/UpdatePreeditText
//! 신호를 받아 출력한다. IBusText 직렬화가 실제로 통하는지(데몬/엔진이 죽지 않는지)
//! 확인하는 용도.
//!
//! 사용법: `cargo run -p presguel-ibus --example drive -- "kf kfhf"`
//! (공백은 그대로 space 키로 전송)

use std::path::PathBuf;
use std::time::Duration;

use zbus::connection::Builder;
use zbus::zvariant::{OwnedObjectPath, Value};

fn ibus_address() -> Result<String, String> {
    if let Ok(a) = std::env::var("IBUS_ADDRESS") {
        if !a.is_empty() {
            return Ok(a);
        }
    }
    let config = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".config")))
        .ok_or("no HOME")?;
    let machine_id = std::fs::read_to_string("/etc/machine-id")
        .or_else(|_| std::fs::read_to_string("/var/lib/dbus/machine-id"))
        .map_err(|e| e.to_string())?
        .trim()
        .to_string();
    let (host, disp) = if let Ok(w) = std::env::var("WAYLAND_DISPLAY") {
        ("unix".to_string(), w)
    } else {
        let d = std::env::var("DISPLAY").unwrap_or_default();
        let (_h, r) = d.split_once(':').unwrap_or(("", "0"));
        ("unix".to_string(), r.split('.').next().unwrap_or("0").to_string())
    };
    let file = config.join("ibus").join("bus").join(format!("{machine_id}-{host}-{disp}"));
    let body = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    for line in body.lines() {
        if let Some(v) = line.trim().strip_prefix("IBUS_ADDRESS=") {
            return Ok(v.trim().to_string());
        }
    }
    Err("IBUS_ADDRESS not found".into())
}

/// IBusText variant 에서 본문 문자열(필드 2)을 뽑는다.
fn ibus_text_string(v: &Value<'_>) -> String {
    if let Value::Structure(s) = v {
        if let Some(Value::Str(t)) = s.fields().get(2) {
            return t.as_str().to_string();
        }
    }
    String::new()
}

/// 변수(v) 래핑을 한 겹 벗긴다.
fn unwrap_variant<'a>(v: &'a Value<'a>) -> &'a Value<'a> {
    match v {
        Value::Value(b) => b.as_ref(),
        other => other,
    }
}

/// IBusProperty variant 에서 symbol(마지막 필드, IBusText)의 글자를 뽑는다.
fn prop_symbol(prop: &Value<'_>) -> String {
    if let Value::Structure(s) = prop {
        if let Some(field) = s.fields().get(11) {
            return ibus_text_string(unwrap_variant(field));
        }
    }
    String::new()
}

/// 16/10진 정수 파싱.
fn parse_int(s: &str) -> Option<u32> {
    s.strip_prefix("0x").and_then(|h| u32::from_str_radix(h, 16).ok()).or_else(|| s.parse().ok())
}

/// 입력 인자를 (라벨, keyval, state) 목록으로 만든다.
/// 기본: 각 문자를 그 코드포인트 keyval(state 0)로.
/// `--raw <tok>...`: 각 토큰은 `keyval` 또는 `keyval:state`(둘 다 16/10진).
fn parse_keys(args: &[String]) -> Vec<(String, u32, u32)> {
    if args.first().map(String::as_str) == Some("--raw") {
        args[1..]
            .iter()
            .filter_map(|t| {
                let (kvs, sts) = t.split_once(':').unwrap_or((t.as_str(), "0"));
                let kv = parse_int(kvs)?;
                let st = parse_int(sts)?;
                Some((format!("0x{kv:04x}:0x{st:x}"), kv, st))
            })
            .collect()
    } else {
        let s = args.first().cloned().unwrap_or_else(|| "kf kfhf".to_string());
        s.chars().map(|c| (format!("{c:?}"), c as u32, 0u32)).collect()
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli: Vec<String> = std::env::args().skip(1).collect();
    let keys = parse_keys(&cli);

    let addr = ibus_address()?;
    let conn = Builder::address(addr.as_str())?.build().await?;

    // Factory.CreateEngine("presguel")
    let factory = zbus::Proxy::new(
        &conn,
        "org.freedesktop.IBus.Presguel",
        "/org/freedesktop/IBus/Factory",
        "org.freedesktop.IBus.Factory",
    )
    .await?;
    let engine_path: OwnedObjectPath = factory.call("CreateEngine", &"presguel").await?;
    println!("engine path = {}", engine_path.as_str());

    let engine = zbus::Proxy::new(
        &conn,
        "org.freedesktop.IBus.Presguel",
        engine_path.clone(),
        "org.freedesktop.IBus.Engine",
    )
    .await?;

    // 신호 구독 → 백그라운드로 출력.
    let mut commit = engine.receive_signal("CommitText").await?;
    let mut preedit = engine.receive_signal("UpdatePreeditText").await?;
    let mut regprop = engine.receive_signal("RegisterProperties").await?;
    let mut updprop = engine.receive_signal("UpdateProperty").await?;
    let mut fwd = engine.receive_signal("ForwardKeyEvent").await?;
    let commit_log = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let clog = commit_log.clone();
    tokio::spawn(async move {
        use futures_util::StreamExt;
        loop {
            tokio::select! {
                Some(msg) = commit.next() => {
                    let body = msg.body();
                    if let Ok((v,)) = body.deserialize::<(Value,)>() {
                        let t = ibus_text_string(&v);
                        println!("  ← CommitText {t:?}");
                        clog.lock().unwrap().push_str(&t);
                    }
                }
                Some(msg) = preedit.next() => {
                    let body = msg.body();
                    if let Ok((v, _c, vis, _m)) = body.deserialize::<(Value, u32, bool, u32)>() {
                        println!("  ← UpdatePreeditText {:?} (visible={vis})", ibus_text_string(&v));
                    }
                }
                Some(msg) = updprop.next() => {
                    let body = msg.body();
                    if let Ok((v,)) = body.deserialize::<(Value,)>() {
                        println!("  ← UpdateProperty symbol={:?}", prop_symbol(&v));
                    }
                }
                Some(msg) = fwd.next() => {
                    let body = msg.body();
                    if let Ok((kv, _kc, st)) = body.deserialize::<(u32, u32, u32)>() {
                        let ch = char::from_u32(kv).unwrap_or('?');
                        println!("  ← ForwardKeyEvent keyval=0x{kv:02x} ({ch:?}) state=0x{st:x}");
                    }
                }
                Some(msg) = regprop.next() => {
                    let _ = msg; // 등록 신호는 도착만 확인
                    println!("  ← RegisterProperties");
                }
                else => break,
            }
        }
    });

    let _: () = engine.call("FocusIn", &()).await.unwrap_or(());

    for (label, keyval, state) in &keys {
        println!("→ key {label}");
        let handled: bool = engine.call("ProcessKeyEvent", &(*keyval, 0u32, *state)).await?;
        println!("  handled={handled}");
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
    // 남은 조합 확정
    let _: () = engine.call("FocusOut", &()).await.unwrap_or(());
    tokio::time::sleep(Duration::from_millis(150)).await;

    println!("\n총 확정 문자열 = {:?}", commit_log.lock().unwrap());
    Ok(())
}
