#![allow(
    dead_code,
    reason = "each integration test compiles this shared module separately"
)]

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, header},
    response::Response,
};
use base64::{Engine, engine::general_purpose};
use serde_json::Value;
use teatro::{
    api,
    config::{
        AppConfig, AuthConfig, DownloadArchiveConfig, GogImportConfig, IgdbConfig,
        LanDiscoveryConfig, LogFormat, RommSourceConfig, UploadConfig,
    },
    domain::user::{User, UserRole},
    repositories::users,
    services::auth,
    state::AppState,
};
use tempfile::TempDir;

pub const REMOVED_PLATFORM_SLUGS: &[&str] = &[
    "flash",
    "ags",
    "macintosh",
    "arcade",
    "astrocde",
    "sufami",
    "bbcmicro",
    "cavestory",
    "chailove",
    "megaduck",
    "desktop",
    "dragon32",
    "easyrpg",
    "epic",
    "channelf",
    "fmtowns",
    "android",
    "pc",
    "zmachine",
    "j2me",
    "kodi",
    "lutris",
    "lutro",
    "mugen",
    "odyssey2",
    "moonlight",
    "mess",
    "gameandwatch",
    "pokemini",
    "openbor",
    "multivision",
    "palm",
    "videopac",
    "ports",
    "samcoupe",
    "zx81",
    "neogeocdjp",
    "solarus",
    "spectravideo",
    "stratagus",
    "symbian",
    "coco",
    "trs-80",
    "oric",
    "tanodragon",
    "ti99",
    "moto",
    "to8",
    "tic80",
    "uzebox",
    "steam",
    "vectrex",
    "supervision",
    "mame-advmame",
    "amstradcpc",
    "gx4000",
    "apple2",
    "apple2gs",
    "sg-1000",
    "model2",
    "model3",
    "naomi",
    "naomigd",
    "atari2600",
    "atari5200",
    "atari7800",
    "atari800",
    "atarixe",
    "atarist",
    "amiga",
    "amiga600",
    "amiga1200",
    "c64",
    "cdtv",
    "vic20",
    "pc88",
    "pc98",
    "dos",
    "msx",
    "msx1",
    "msx2",
    "msxturbor",
    "x1",
    "x68000",
    "zxspectrum",
    "atomiswave",
    "cps",
    "daphne",
    "fba",
    "fbneo",
    "mame",
    "mame-mame4all",
    "doom",
    "scummvm",
    "pico8",
];

pub fn test_config(temp_dir: &TempDir) -> AppConfig {
    let root = temp_dir.path();
    AppConfig {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        database_url: format!("sqlite://{}", root.join("teatro.sqlite3").display()),
        data_dir: root.join("data"),
        default_library_root: root.join("roms"),
        asset_root: root.join("assets"),
        max_upload_bytes: 1024 * 1024,
        uploads: UploadConfig::default(),
        download_archives: DownloadArchiveConfig {
            free_space_margin_bytes: 0,
            ..DownloadArchiveConfig::default()
        },
        gog_import: GogImportConfig::default(),
        romm_source: RommSourceConfig::default(),
        log_format: LogFormat::Compact,
        lan_discovery: LanDiscoveryConfig::default(),
        auth: AuthConfig::default(),
        igdb: IgdbConfig::default(),
    }
}

#[derive(Debug, Clone)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

pub struct TestApp {
    pub temp_dir: TempDir,
    pub state: AppState,
    pub router: Router,
}

impl TestApp {
    pub async fn new() -> Self {
        Self::with_config(|_| {}).await
    }

    pub async fn with_config(configure: impl FnOnce(&mut AppConfig)) -> Self {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let mut config = test_config(&temp_dir);
        configure(&mut config);
        let state = AppState::initialize(config).await.unwrap();
        let router = api::router(state.clone());
        Self {
            temp_dir,
            state,
            router,
        }
    }

    pub async fn seed_user(&self, username: &str, password: &str, role: UserRole) -> Credentials {
        seed_user_record(&self.state, username, password, role).await;
        Credentials {
            username: username.to_string(),
            password: password.to_string(),
        }
    }

    pub async fn seed_admin(&self, username: &str, password: &str) -> Credentials {
        self.seed_user(username, password, UserRole::Admin).await
    }

    pub async fn seed_readonly(&self, username: &str, password: &str) -> Credentials {
        self.seed_user(username, password, UserRole::ReadOnly).await
    }

    pub async fn seed_rom(&self, platform_slug: &str, name: &str, slug: &str) -> i64 {
        seed_rom(&self.state, platform_slug, name, slug).await
    }
}

pub async fn seed_user_record(
    state: &AppState,
    username: &str,
    password: &str,
    role: UserRole,
) -> User {
    let password_hash = auth::hash_password(password).unwrap();
    users::create(state.db(), username, &password_hash, role)
        .await
        .unwrap()
}

pub async fn seed_rom(state: &AppState, platform_slug: &str, name: &str, slug: &str) -> i64 {
    let platform_id: i64 = sqlx::query_scalar("SELECT id FROM platforms WHERE slug = ?")
        .bind(platform_slug)
        .fetch_one(state.db())
        .await
        .unwrap();
    sqlx::query("INSERT INTO roms (platform_id, name, slug, regions_json) VALUES (?, ?, ?, '[]')")
        .bind(platform_id)
        .bind(name)
        .bind(slug)
        .execute(state.db())
        .await
        .unwrap()
        .last_insert_rowid()
}

pub fn basic_auth(username: &str, password: &str) -> String {
    let encoded = general_purpose::STANDARD.encode(format!("{username}:{password}"));
    format!("Basic {encoded}")
}

pub fn empty_request(method: &str, uri: &str, auth: Option<(&str, &str)>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some((username, password)) = auth {
        builder = builder.header(header::AUTHORIZATION, basic_auth(username, password));
    }
    builder.body(Body::empty()).unwrap()
}

pub fn json_request(
    method: &str,
    uri: &str,
    auth: Option<(&str, &str)>,
    body: Value,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some((username, password)) = auth {
        builder = builder.header(header::AUTHORIZATION, basic_auth(username, password));
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

pub fn multipart_request(
    method: &str,
    uri: &str,
    auth: Option<(&str, &str)>,
    fields: &[(&str, &str)],
    files: &[(&str, &str, &[u8])],
) -> Request<Body> {
    let boundary = "teatro-shared-test-boundary";
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")
                .as_bytes(),
        );
    }
    for (field_name, file_name, bytes) in files {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"{field_name}\"; filename=\"{file_name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

    let mut builder = Request::builder().method(method).uri(uri).header(
        header::CONTENT_TYPE,
        format!("multipart/form-data; boundary={boundary}"),
    );
    if let Some((username, password)) = auth {
        builder = builder.header(header::AUTHORIZATION, basic_auth(username, password));
    }
    builder.body(Body::from(body)).unwrap()
}

pub async fn response_json(response: Response) -> Value {
    let bytes = response_bytes(response).await;
    serde_json::from_slice(&bytes).unwrap()
}

pub async fn response_bytes(response: Response) -> axum::body::Bytes {
    to_bytes(response.into_body(), usize::MAX).await.unwrap()
}

pub async fn response_text(response: Response) -> String {
    String::from_utf8(response_bytes(response).await.to_vec()).unwrap()
}
