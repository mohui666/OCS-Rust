use anyhow::{bail, Context, Result};
use ocs_core::{
    bridge::{self, Bridge, Config},
    storage::atomic_write,
};
use std::{path::Path, process::Command};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    match args.get(1).map(String::as_str){
        Some("serve")=>{
            let file=args.get(2).context("用法：ocs-bridge serve /absolute/config.json")?;
            let mut config = Config::load(Path::new(file), None)?;
            if let Ok(key) = std::env::var("OCS_AI_API_KEY") { config.api_key = key; }
            config.validate_ready()?;
            let listener=tokio::net::TcpListener::bind(("127.0.0.1",config.port)).await?;config.port=listener.local_addr()?.port();
            eprintln!("OCS Rust bridge listening on 127.0.0.1:{}",config.port);
            tokio::select! {
                result = async { axum::serve(listener,bridge::router(Bridge::new(config)?)).await.map_err(anyhow::Error::from) } => result?,
                _ = shutdown_signal() => {}
            }
        },
        Some("check")=>{let config=Config::load(Path::new(args.get(2).context("缺少配置文件")?),None)?;config.validate()?;println!("配置有效；未调用模型。超时：{}",if config.timeout().is_none(){"无限等待"}else{"有限等待"});},
        Some("service")=>service(args.get(2).map(String::as_str).unwrap_or("status"),args.get(3).map(Path::new))?,
        _=>bail!("用法：ocs-bridge serve|check CONFIG，或 service install CONFIG / start / stop / status"),
    }
    Ok(())
}
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("SIGTERM handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
fn command(program: &str, args: &[&str]) -> Result<()> {
    let s = Command::new(program).args(args).status()?;
    if !s.success() {
        bail!("{program} 执行失败：{s}")
    }
    Ok(())
}
fn service(action: &str, config: Option<&Path>) -> Result<()> {
    let label = "local.ocs-rust-bridge";
    let home = dirs::home_dir().context("没有用户目录")?;
    let executable = std::env::current_exe()?.canonicalize()?;
    let install = action == "install";
    let config = if install {
        Some(
            config
                .context("安装服务需要绝对路径配置文件")?
                .canonicalize()?,
        )
    } else {
        None
    };
    if let Some(c) = &config {
        let v: Config = serde_json::from_slice(&std::fs::read(c)?)?;
        v.validate()?;
    }
    #[cfg(target_os = "macos")]
    {
        let domain = format!("gui/{}", unsafe { libc::getuid() });
        let target = format!("{domain}/{label}");
        let file = home
            .join("Library/LaunchAgents")
            .join(format!("{label}.plist"));
        if install {
            let config = config.as_ref().unwrap();
            let logs = config.parent().unwrap().join("logs");
            std::fs::create_dir_all(&logs)?;
            let mut d = plist::Dictionary::new();
            d.insert("Label".into(), label.into());
            d.insert(
                "ProgramArguments".into(),
                plist::Value::Array(vec![
                    executable.to_string_lossy().as_ref().into(),
                    "serve".into(),
                    config.to_string_lossy().as_ref().into(),
                ]),
            );
            d.insert("RunAtLoad".into(), true.into());
            d.insert("KeepAlive".into(), true.into());
            d.insert("ThrottleInterval".into(), plist::Value::Integer(10.into()));
            d.insert(
                "StandardErrorPath".into(),
                logs.join("rust-bridge.log")
                    .to_string_lossy()
                    .as_ref()
                    .into(),
            );
            d.insert(
                "StandardOutPath".into(),
                logs.join("rust-bridge-stdout.log")
                    .to_string_lossy()
                    .as_ref()
                    .into(),
            );
            let value = plist::Value::Dictionary(d);
            let mut bytes = vec![];
            value.to_writer_xml(&mut bytes)?;
            if file.exists() && plist::Value::from_file(&file)? != value {
                bail!("已有不同配置的同名 Rust 服务，未覆盖")
            }
            atomic_write(&file, &bytes)?;
            println!(
                "服务定义已写入 {}；执行 service start 启动。",
                file.display()
            );
            return Ok(());
        }
        match action {
            "start" => {
                if Command::new("launchctl")
                    .args(["print", &target])
                    .output()?
                    .status
                    .success()
                {
                    command("launchctl", &["kickstart", &target])?
                } else {
                    command(
                        "launchctl",
                        &["bootstrap", &domain, file.to_str().context("服务路径无效")?],
                    )?
                }
            }
            "stop" => command("launchctl", &["bootout", &target])?,
            "status" => command("launchctl", &["print", &target])?,
            _ => bail!("未知服务操作"),
        }
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        let unit = format!("{label}.service");
        let file = home.join(".config/systemd/user").join(&unit);
        if install {
            let quote = |s: &str| {
                format!(
                    "\"{}\"",
                    s.replace('\\', "\\\\")
                        .replace('"', "\\\"")
                        .replace('%', "%%")
                )
            };
            let text=format!("[Unit]\nDescription=OCS Rust Bridge\n[Service]\nExecStart={} serve {}\nRestart=on-failure\n[Install]\nWantedBy=default.target\n",quote(executable.to_str().unwrap()),quote(config.as_ref().unwrap().to_str().unwrap()));
            if file.exists() && std::fs::read_to_string(&file)? != text {
                bail!("已有不同配置的同名服务，未覆盖")
            }
            atomic_write(&file, text.as_bytes())?;
            return command("systemctl", &["--user", "daemon-reload"]);
        }
        return match action {
            "start" => command("systemctl", &["--user", "start", &unit]),
            "stop" => command("systemctl", &["--user", "stop", &unit]),
            "status" => command("systemctl", &["--user", "status", &unit]),
            _ => bail!("未知服务操作"),
        };
    }
    #[cfg(windows)]
    {
        if install {
            if Command::new("schtasks")
                .args(["/Query", "/TN", label])
                .output()?
                .status
                .success()
            {
                bail!("已有同名任务，未覆盖")
            }
            let run = format!(
                "\"{}\" serve \"{}\"",
                executable.display(),
                config.as_ref().unwrap().display()
            );
            return command(
                "schtasks",
                &[
                    "/Create", "/TN", label, "/TR", &run, "/SC", "ONLOGON", "/RL", "LIMITED",
                ],
            );
        }
        return match action {
            "start" => command("schtasks", &["/Run", "/TN", label]),
            "stop" => command("schtasks", &["/End", "/TN", label]),
            "status" => command("schtasks", &["/Query", "/TN", label]),
            _ => bail!("未知服务操作"),
        };
    }
    #[allow(unreachable_code)]
    {
        bail!("此平台暂不支持用户服务")
    }
}
