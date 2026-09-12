use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

use tusher_core::identity::DeviceIdentity;
use tusher_metadata::MetadataService;
use tusher_network::manager::ConnectionManager;
use tusher_sync::SyncCoordinator;
use tusher_transfer::TransferService;

use tusher_desktop::shell::{install_context_menu, is_context_menu_installed, uninstall_context_menu};
use tusher_desktop::tray::{run_tray, TrayCallbacks};
use tusher_desktop::web::{start_web_server, DesktopState};

#[derive(Parser, Debug)]
#[command(name = "tusher-desktop")]
#[command(author, version, about = "T.U.S.H.E.R - Desktop Product & Mesh Control Node", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(short, long, default_value = "TusherDesktop")]
    name: String,

    #[arg(short, long, default_value_t = 42424)]
    port: u16,

    #[arg(long, default_value_t = 42425)]
    discovery_port: u16,

    #[arg(long, default_value_t = 42950)]
    web_port: u16,

    #[arg(short, long, default_value = "./.tusher_desktop")]
    data_dir: PathBuf,

    #[arg(long, default_value_t = false)]
    no_tray: bool,

    #[arg(long, default_value_t = false)]
    open: bool,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Share a file or directory via T.U.S.H.E.R (invoked from Explorer context menu)
    Share {
        path: PathBuf,
    },
    /// Manage Windows Explorer context menu integration
    Shell {
        #[command(subcommand)]
        action: ShellAction,
    },
}

#[derive(Subcommand, Debug)]
enum ShellAction {
    /// Install Windows Explorer context menu ("Share via T.U.S.H.E.R")
    Install,
    /// Remove Windows Explorer context menu
    Uninstall,
    /// Check if Windows Explorer context menu is installed
    Status,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .with_target(false)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    let cli = Cli::parse();

    // Handle subcommands
    if let Some(cmd) = cli.command {
        match cmd {
            Commands::Share { path } => {
                handle_share_action(&path, cli.web_port).await?;
                return Ok(());
            }
            Commands::Shell { action } => {
                match action {
                    ShellAction::Install => {
                        install_context_menu(None)?;
                        println!("✓ Successfully installed Windows Explorer context menu!");
                    }
                    ShellAction::Uninstall => {
                        uninstall_context_menu()?;
                        println!("✓ Successfully uninstalled Windows Explorer context menu!");
                    }
                    ShellAction::Status => {
                        let installed = is_context_menu_installed();
                        println!("Explorer Context Menu Installed: {}", installed);
                    }
                }
                return Ok(());
            }
        }
    }

    // Default: Run desktop node with tray and embedded web dashboard
    let identity = Arc::new(DeviceIdentity::load_or_create(&cli.data_dir, &cli.name)?);

    let staging_dir = cli.data_dir.join(".staging");
    let downloads_dir = cli.data_dir.join("downloads");
    std::fs::create_dir_all(&downloads_dir)?;

    let transfer_service = Arc::new(TransferService::new(&staging_dir, &downloads_dir).await?);
    let db_path = cli.data_dir.join("tusher_metadata.db");
    let metadata_service = Arc::new(MetadataService::open(&db_path)?);

    let manager = Arc::new(ConnectionManager::new(
        Arc::clone(&identity),
        cli.port,
        cli.discovery_port,
    ));

    let sync_coordinator = Arc::new(SyncCoordinator::new(
        identity.node_id().clone(),
        Arc::clone(&metadata_service),
        Arc::clone(&transfer_service),
        Arc::clone(&manager),
        std::time::Duration::from_millis(300),
    ));

    // Register existing folders from metadata database
    if let Ok(existing_folders) = metadata_service.list_folders().await {
        for f in existing_folders {
            let _ = sync_coordinator.register_folder(&f.folder_id, &f.local_path).await;
        }
    }

    let _maint_handle = Arc::clone(&manager).start().await?;
    let _sync_handle = Arc::clone(&sync_coordinator).start().await?;

    let sync_paused = Arc::new(AtomicBool::new(false));

    let desktop_state = DesktopState {
        identity: Arc::clone(&identity),
        manager: Arc::clone(&manager),
        metadata_service: Arc::clone(&metadata_service),
        transfer_service: Arc::clone(&transfer_service),
        sync_coordinator: Arc::clone(&sync_coordinator),
        sync_paused: Arc::clone(&sync_paused),
        downloads_dir: downloads_dir.clone(),
        tcp_port: cli.port,
        discovery_port: cli.discovery_port,
        web_port: cli.web_port,
    };

    // Spawn Axum Web Server
    let web_state = desktop_state.clone();
    tokio::spawn(async move {
        if let Err(e) = start_web_server(web_state).await {
            tracing::error!("Web dashboard error: {}", e);
        }
    });

    println!("\n============================================================");
    println!("     T.U.S.H.E.R - Desktop Product & Mesh Control Node      ");
    println!("============================================================");
    println!(" Node Name:       {}", identity.node_name());
    println!(" Node ID:         {}", identity.node_id());
    println!(" Platform:        {}", identity.platform());
    println!(" TCP Mesh Port:   {}", cli.port);
    println!(" UDP Discovery:   {}", cli.discovery_port);
    println!(" Web Dashboard:   http://127.0.0.1:{}", cli.web_port);
    println!(" Downloads Dir:   {}", downloads_dir.display());
    println!(" Context Menu:    {}", if is_context_menu_installed() { "Installed" } else { "Not Installed" });
    println!("============================================================\n");

    let dashboard_url = format!("http://127.0.0.1:{}", cli.web_port);

    if cli.open {
        let _ = opener::open(&dashboard_url);
    }

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);

    let _tray_handle = if !cli.no_tray {
        let url_copy = dashboard_url.clone();
        let down_copy = downloads_dir.clone();
        let sync_paused_copy = Arc::clone(&sync_paused);
        let shutdown_tx_copy = shutdown_tx.clone();

        let callbacks = TrayCallbacks {
            on_open_dashboard: Box::new(move || {
                let _ = opener::open(&url_copy);
            }),
            on_open_downloads: Box::new(move || {
                let _ = opener::open(&down_copy);
            }),
            on_toggle_sync: Box::new(move || {
                let current = sync_paused_copy.load(std::sync::atomic::Ordering::SeqCst);
                let new_val = !current;
                sync_paused_copy.store(new_val, std::sync::atomic::Ordering::SeqCst);
                info!("Sync toggled: paused = {}", new_val);
                new_val
            }),
            on_toggle_context_menu: Box::new(|| {
                let installed = is_context_menu_installed();
                if installed {
                    let _ = uninstall_context_menu();
                    info!("Explorer context menu removed");
                    false
                } else {
                    let _ = install_context_menu(None);
                    info!("Explorer context menu installed");
                    true
                }
            }),
            on_exit: Box::new(move || {
                info!("Exit requested from system tray");
                let _ = shutdown_tx_copy.try_send(());
            }),
        };

        Some(run_tray(callbacks)?)
    } else {
        None
    };

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("Ctrl+C received, shutting down T.U.S.H.E.R Desktop...");
        }
        _ = shutdown_rx.recv() => {
            info!("Tray exit signal received, shutting down...");
        }
    }

    Ok(())
}

async fn handle_share_action(path: &Path, web_port: u16) -> anyhow::Result<()> {
    println!(">>> T.U.S.H.E.R Share: {}", path.display());
    if !path.exists() {
        println!("Error: Path '{}' does not exist.", path.display());
        return Ok(());
    }

    // Check if desktop node is currently running
    let status_url = format!("http://127.0.0.1:{}/api/status", web_port);
    let is_running = match tokio::net::TcpStream::connect(format!("127.0.0.1:{}", web_port)).await {
        Ok(_) => true,
        Err(_) => false,
    };

    let dashboard_url = format!("http://127.0.0.1:{}", web_port);

    if is_running {
        println!("Active T.U.S.H.E.R node detected at {}", status_url);
        if path.is_dir() {
            let folder_id = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "shared_folder".to_string());
            println!("Opening dashboard to add shared folder: '{}'...", folder_id);
            let _ = opener::open(&dashboard_url);
        } else {
            println!("Opening dashboard to share file: '{}'...", path.display());
            let _ = opener::open(&dashboard_url);
        }
    } else {
        println!("T.U.S.H.E.R Desktop is not running. Launching with shared path...");
        let _ = opener::open(path);
    }

    Ok(())
}
