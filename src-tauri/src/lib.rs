use chrono::Utc;
use reqwest::blocking::{Client, Response};
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, File};
use std::io::{self, Read, Seek, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Manager};
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const MAX_SCAN_DEPTH: usize = 6;
const MAX_SCAN_ENTRIES: usize = 3000;
const MAX_UNREAL_PAK_ROOT_SCAN_DEPTH: usize = 9;
const MAX_UNREAL_PAK_ROOTS: usize = 64;
const MAX_DEPENDENCY_DEPTH: usize = 8;
const MAX_CONFIG_READ_BYTES: u64 = 512 * 1024;
const MAX_CONFIG_SCAN_DEPTH: usize = 5;
const MAX_PROFILE_CONFIG_FILES: usize = 500;
const MAX_DOWNLOAD_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_DOWNLOAD_ATTEMPTS: usize = 3;
const DOWNLOAD_RETRY_BASE_DELAY_MS: u64 = 350;
const DOWNLOAD_RETRY_MAX_DELAY_SECS: u64 = 5;
const MAX_ARCHIVE_ENTRIES: usize = 10_000;
const MAX_ARCHIVE_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_ARCHIVE_EXPANDED_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_ARCHIVE_PATH_DEPTH: usize = 32;
const MAX_ARCHIVE_COMPRESSION_RATIO: u64 = 500;
const MAX_IMPORT_SCAN_DEPTH: usize = 32;
const IMPORT_CACHE_MAX_AGE_HOURS: i64 = 24;
const THUNDERSTORE_API_BASE: &str = "https://thunderstore.io/api/experimental/package";
const THUNDERSTORE_COMMUNITY_API_BASE: &str = "https://thunderstore.io/c";
const NEXUS_GRAPHQL_API_BASE: &str = "https://api.nexusmods.com/v2/graphql";
const NEXUS_SITE_BASE: &str = "https://www.nexusmods.com";
// The full discovery query costs roughly 181 complexity points per record.
// Keep batches comfortably below Nexus Mods' 10,000-point query ceiling.
const NEXUS_DISCOVERY_PAGE_SIZE: usize = 40;
const NEXUS_PENDING_DOWNLOAD_TTL_MINUTES: i64 = 30;
const MAX_DISCOVERY_PAGE_SIZE: usize = 50;
const MAX_PROVIDER_CANDIDATES: usize = 16;
const PROFILE_LAUNCH_SUSPENSION_VERSION: u32 = 1;
const PROFILE_RUNTIME_SAMPLE_SIZE: usize = 40;
const PROFILE_RUNTIME_MAX_DEPENDENCY_DEPTH: usize = 5;
const PROFILE_RUNTIME_MIN_SUPPORT: usize = 2;
const PROFILE_ROUTE_KNOWLEDGE_VERSION: u32 = 1;
const PROFILE_ROUTE_SAMPLE_SIZE: usize = 32;
const PROFILE_ROUTE_FULL_TEXT_SAMPLE_SIZE: usize = 18;
const PROFILE_ROUTE_FETCH_CONCURRENCY: usize = 6;
const PROFILE_ROUTE_MIN_SUPPORT: usize = 2;
const PROFILE_ROUTE_MAX_EVIDENCE: usize = 8;
const PROFILE_ROUTE_CACHE_HOURS: i64 = 24 * 7;
const PROFILE_ROUTE_NEGATIVE_CACHE_MINUTES: i64 = 10;
const PROVIDER_MAPPING_CACHE_HOURS: u64 = 12;
const PROVIDER_MAPPING_NEGATIVE_CACHE_MINUTES: u64 = 10;
const GITHUB_API_BASE: &str = "https://api.github.com/repos";
const APP_UPDATE_REPOSITORY: &str = "Chucksterboy/UniLoader";
const BEPINBUILDS_BASE: &str = "https://builds.bepinex.dev";
const BEPINBUILDS_BEPINEX_BE: &str = "https://builds.bepinex.dev/projects/bepinex_be";
const APP_ICON: tauri::image::Image<'static> = tauri::include_image!("./icons/icon.png");
const GAME_DEFINITIONS_JSON: &str = include_str!("game_definitions.json");
const RUNTIME_DEFINITIONS_JSON: &str = include_str!("runtime_definitions.json");
const NEXUS_KEYRING_SERVICE: &str = "UniLoader";
const NEXUS_KEYRING_USER: &str = "nexus-api-key";

static STORE_WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static MUTATION_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static THUNDERSTORE_CACHE: OnceLock<Mutex<HashMap<String, ThunderstoreCacheEntry>>> =
    OnceLock::new();
static PROVIDER_MAPPING_CACHE: OnceLock<Mutex<HashMap<String, ProviderMappingCacheEntry>>> =
    OnceLock::new();
static NEXUS_GAME_ID_CACHE: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
static PROFILE_RUNTIME_INFERENCE_CACHE: OnceLock<
    Mutex<HashMap<String, RuntimeInferenceCacheEntry>>,
> = OnceLock::new();

#[derive(Debug, Clone)]
struct ThunderstoreCacheEntry {
    fetched_at: Instant,
    packages: Vec<ThunderstoreCommunityPackage>,
}

#[derive(Debug, Clone)]
struct ProviderMappingCacheEntry {
    fetched_at: Instant,
    value: Option<String>,
}

#[derive(Debug, Clone)]
struct RuntimeInferenceCacheEntry {
    fetched_at: Instant,
    value: Option<ProviderRuntimeInference>,
}

#[derive(Debug, Clone)]
struct ProviderRuntimeInference {
    runtime_id: String,
    providers: Vec<String>,
    supporting_mods: usize,
    sampled_mods: usize,
}

#[derive(Debug, Clone)]
struct ProviderRouteDocument {
    provider: String,
    mod_id: String,
    mod_name: String,
    text: String,
}

#[derive(Debug, Clone)]
struct InstallRouteCandidate {
    relative_path: String,
    adapter_id: String,
    scopes: Vec<String>,
    excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RouteEvidence {
    provider: String,
    mod_id: String,
    mod_name: String,
    excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LearnedInstallRoute {
    relative_path: String,
    adapter_id: String,
    #[serde(default)]
    scopes: Vec<String>,
    confidence: f64,
    supporting_mods: usize,
    #[serde(default)]
    providers: Vec<String>,
    #[serde(default)]
    evidence: Vec<RouteEvidence>,
    #[serde(default)]
    trusted: bool,
    #[serde(default)]
    package_verified: bool,
    #[serde(default)]
    created: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProfileRouteKnowledge {
    version: u32,
    profile_id: String,
    learned_at: String,
    sampled_mods: usize,
    #[serde(default)]
    providers: Vec<String>,
    #[serde(default)]
    routes: Vec<LearnedInstallRoute>,
    #[serde(default)]
    warnings: Vec<String>,
}

#[derive(Debug, Default)]
struct RouteKnowledgeOutcome {
    expected_routes: Vec<String>,
    created_routes: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameProfile {
    id: String,
    name: String,
    game_path: String,
    #[serde(default)]
    game_id: Option<String>,
    #[serde(default)]
    steam_app_id: Option<String>,
    engine: String,
    loader: String,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProfileInput {
    name: String,
    game_path: String,
    #[serde(default)]
    game_id: Option<String>,
    #[serde(default)]
    steam_app_id: Option<String>,
    engine: String,
    loader: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamGameRecord {
    app_id: String,
    name: String,
    install_dir: String,
    library_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectionSignal {
    label: String,
    path: String,
    weight: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameDetectionResult {
    game_path: String,
    game_id: Option<String>,
    engine: String,
    loader: String,
    recommended_loader: String,
    engine_confidence: f64,
    loader_confidence: f64,
    loader_installed: bool,
    expected_mod_folders: Vec<String>,
    created_mod_folders: Vec<String>,
    signals: Vec<DetectionSignal>,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveEntry {
    path: String,
    logical_path: String,
    size: u64,
    is_directory: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThunderstoreManifest {
    name: String,
    #[serde(alias = "versionNumber")]
    version_number: String,
    #[serde(default, alias = "websiteUrl")]
    website_url: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    dependencies: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencySpec {
    id: String,
    name: String,
    version: Option<String>,
    provider: String,
    required: bool,
    status: String,
    source: Option<String>,
    notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallPreflightResult {
    dependencies: Vec<DependencySpec>,
    missing_dependencies: Vec<DependencySpec>,
    confirmation_required: bool,
}

#[derive(Debug, Clone)]
struct ThunderstorePackageRef {
    namespace: String,
    name: String,
    version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ThunderstorePackageResponse {
    latest: ThunderstoreVersion,
}

#[derive(Debug, Deserialize)]
struct ThunderstoreMarkdownResponse {
    #[serde(default)]
    markdown: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ThunderstoreVersion {
    version_number: String,
    full_name: String,
    download_url: String,
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default)]
    description: String,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    website_url: Option<String>,
    #[serde(default)]
    is_active: bool,
    #[serde(default)]
    file_size: Option<u64>,
    #[serde(default)]
    date_created: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ThunderstoreCommunityPackage {
    name: String,
    full_name: String,
    owner: String,
    #[serde(default)]
    package_url: Option<String>,
    #[serde(default)]
    rating_score: i64,
    #[serde(default)]
    is_deprecated: bool,
    #[serde(default)]
    has_nsfw_content: bool,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    date_created: Option<String>,
    #[serde(default)]
    date_updated: Option<String>,
    #[serde(default)]
    versions: Vec<ThunderstoreVersion>,
}

#[derive(Debug, Deserialize)]
struct NexusGraphqlResponse {
    data: Option<NexusGraphqlData>,
    #[serde(default)]
    errors: Vec<NexusGraphqlError>,
}

#[derive(Debug, Deserialize)]
struct NexusGraphqlData {
    mods: NexusModPage,
}

#[derive(Debug, Deserialize)]
struct NexusGraphqlError {
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NexusModPage {
    #[serde(default)]
    nodes: Vec<NexusModNode>,
    #[serde(default)]
    total_count: usize,
}

#[derive(Debug, Deserialize)]
struct NexusGamesGraphqlResponse {
    data: Option<NexusGamesGraphqlData>,
    #[serde(default)]
    errors: Vec<NexusGraphqlError>,
}

#[derive(Debug, Deserialize)]
struct NexusGamesGraphqlData {
    games: NexusGamePage,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NexusGamePage {
    #[serde(default)]
    nodes: Vec<NexusGameNode>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NexusGameNode {
    #[serde(default)]
    id: Option<u64>,
    name: String,
    domain_name: String,
}

#[derive(Debug, Deserialize)]
struct NexusGameDetails {
    id: u64,
    #[serde(default)]
    domain_name: String,
}

#[derive(Debug, Deserialize)]
struct NexusFilesResponse {
    #[serde(default)]
    files: Vec<NexusFileRecord>,
}

#[derive(Debug, Clone, Deserialize)]
struct NexusFileRecord {
    file_id: u64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    category_name: Option<String>,
    #[serde(default)]
    is_primary: Option<bool>,
    #[serde(default)]
    uploaded_timestamp: Option<u64>,
    #[serde(default)]
    uploaded_time: Option<String>,
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    size_kb: Option<u64>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct NexusUserValidation {
    #[serde(default)]
    is_premium: bool,
}

#[derive(Debug, Deserialize)]
struct NexusDownloadLink {
    #[serde(default, rename = "URI", alias = "uri")]
    uri: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PendingNexusDownload {
    profile_id: String,
    domain: String,
    mod_id: u64,
    file_id: u64,
    version: Option<String>,
    provider_game_id: String,
    created_at: i64,
}

#[derive(Debug, Clone)]
struct NexusNxmLink {
    domain: String,
    mod_id: u64,
    file_id: u64,
    key: String,
    expires: i64,
    user_id: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NexusModNode {
    mod_id: Option<u64>,
    name: Option<String>,
    summary: Option<String>,
    author: Option<String>,
    category: Option<String>,
    version: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    endorsements: i64,
    file_size: Option<u64>,
    picture_url: Option<String>,
    thumbnail_url: Option<String>,
    thumbnail_large_url: Option<String>,
    #[serde(default)]
    direct_download_enabled: bool,
    #[serde(default)]
    adult_content: bool,
    status: Option<String>,
    #[serde(default)]
    mod_requirements: Option<NexusModRequirements>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct NexusModDetails {
    #[serde(default)]
    name: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    picture_url: Option<String>,
    #[serde(default)]
    thumbnail_url: Option<String>,
    #[serde(default)]
    thumbnail_large_url: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NexusModRequirements {
    #[serde(default)]
    nexus_requirements: NexusRequirementPage,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NexusRequirementPage {
    #[serde(default)]
    nodes: Vec<NexusRequirement>,
    #[serde(default)]
    total_count: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NexusRequirement {
    #[serde(default)]
    external_requirement: bool,
    #[serde(default)]
    game_id: String,
    #[serde(default)]
    mod_id: String,
    #[serde(default)]
    mod_name: String,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineModRecord {
    id: String,
    provider: String,
    provider_label: String,
    game_id: Option<String>,
    provider_game_id: Option<String>,
    name: String,
    owner: String,
    version: String,
    description: String,
    categories: Vec<String>,
    downloads: u64,
    rating_score: i64,
    dependency_count: usize,
    file_size: Option<u64>,
    icon_url: Option<String>,
    package_url: Option<String>,
    website_url: Option<String>,
    installed: bool,
    created_at: Option<String>,
    updated_at: Option<String>,
    install_supported: bool,
    install_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineModFileOption {
    id: String,
    name: String,
    version: Option<String>,
    category: Option<String>,
    description: Option<String>,
    file_name: Option<String>,
    file_size: Option<u64>,
    uploaded_at: Option<String>,
    primary: bool,
    action: String,
    download_page_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryPage {
    items: Vec<OnlineModRecord>,
    total: usize,
    page: usize,
    page_size: usize,
    has_more: bool,
}

#[derive(Debug, Deserialize)]
struct GithubReleaseResponse {
    tag_name: String,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    assets: Vec<GithubReleaseAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubReleaseAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Clone)]
struct ReleaseDependencyRef {
    source_key: String,
    display_name: String,
    download_url: String,
    version: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GameDefinition {
    id: String,
    display_name: String,
    #[serde(default)]
    engine: Option<String>,
    #[serde(default)]
    steam_app_ids: Vec<String>,
    #[serde(default)]
    executable_names: Vec<String>,
    #[serde(default)]
    path_markers: Vec<String>,
    #[serde(default)]
    supported_adapters: Vec<String>,
    #[serde(default)]
    bootstrap_runtimes: Vec<String>,
    #[serde(default)]
    config_roots: Vec<String>,
    #[serde(default)]
    native_script_roots: Vec<String>,
    #[serde(default)]
    thunderstore_community: Option<String>,
    #[serde(default)]
    nexus_game_domain: Option<String>,
    #[serde(default)]
    curseforge_game_id: Option<String>,
    #[serde(default)]
    runtime_dependencies: HashMap<String, RuntimeDependencyDefinition>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeDependencyDefinition {
    id: String,
    name: String,
    provider: String,
    source: String,
    #[serde(default)]
    notes: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeDefinition {
    id: String,
    dependency: RuntimeDependencyDefinition,
    #[serde(default)]
    profile_engines: Vec<String>,
    #[serde(default)]
    profile_loaders: Vec<String>,
    #[serde(default)]
    provider_packages: Vec<RuntimeProviderPackageDefinition>,
    #[serde(default)]
    detection_rules: Vec<RuntimeDetectionRule>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeProviderPackageDefinition {
    provider: String,
    #[serde(default)]
    namespace_patterns: Vec<String>,
    #[serde(default)]
    package_patterns: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RuntimeDetectionRule {
    #[serde(default)]
    all: Vec<String>,
    #[serde(default)]
    any: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallMapping {
    source_path: String,
    target_root: String,
    target_relative_path: String,
    reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallPlan {
    adapter_id: String,
    adapter_name: String,
    confidence: f64,
    summary: String,
    mappings: Vec<InstallMapping>,
    dependencies: Vec<DependencySpec>,
    warnings: Vec<String>,
    requires_confirmation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageIdentity {
    provider: String,
    package_id: Option<String>,
    version: Option<String>,
    provider_game_id: Option<String>,
    mod_types: Vec<String>,
    dependencies: Vec<String>,
    evidence: Vec<String>,
    confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompatibilityResult {
    status: String,
    reason: String,
    confidence: f64,
    game_id: Option<String>,
    provider_game_id: Option<String>,
    detected_mod_types: Vec<String>,
    supported_mod_types: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveAnalysis {
    archive_path: String,
    archive_name: String,
    entries: Vec<ArchiveEntry>,
    manifest: Option<ThunderstoreManifest>,
    package_identity: PackageIdentity,
    compatibility: CompatibilityResult,
    plans: Vec<InstallPlan>,
    recommended_plan: Option<InstallPlan>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallRequest {
    profile_id: String,
    archive_path: String,
    #[serde(default)]
    archive_name: Option<String>,
    package_identity: Option<PackageIdentity>,
    plan: InstallPlan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallResult {
    profile_id: String,
    archive_path: String,
    installed_mod_id: String,
    installed_at: String,
    files_written: Vec<String>,
    backups_written: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledModRecord {
    id: String,
    profile_id: String,
    archive_path: String,
    archive_name: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    package_id: Option<String>,
    #[serde(default)]
    dependency_string: Option<String>,
    #[serde(default)]
    icon_url: Option<String>,
    adapter_id: String,
    summary: String,
    installed_at: String,
    files_written: Vec<String>,
    backups_written: Vec<String>,
    #[serde(default)]
    written_file_hashes: HashMap<String, String>,
    dependencies: Vec<DependencySpec>,
    #[serde(default)]
    config_files: Vec<String>,
    #[serde(default)]
    runtime_id: Option<String>,
    #[serde(default)]
    externally_managed: bool,
    #[serde(default = "default_enabled")]
    enabled: bool,
    #[serde(default = "default_last_status")]
    last_status: String,
    #[serde(default)]
    plan: Option<InstallPlan>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModConfigEntry {
    section: Option<String>,
    key: String,
    value: String,
    value_type: Option<String>,
    default_value: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModConfigFile {
    path: String,
    file_name: String,
    entries: Vec<ModConfigEntry>,
    raw_preview: String,
    warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateModConfigValueInput {
    profile_id: String,
    file_path: String,
    section: Option<String>,
    key: String,
    value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModActionResult {
    profile_id: String,
    installed_mod_id: String,
    status: String,
    files_changed: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileModToggleResult {
    profile_id: String,
    enabled: bool,
    changed_mods: usize,
    files_changed: Vec<String>,
    warnings: Vec<String>,
    installed_mods: Vec<InstalledModRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SuspendedLaunchFile {
    destination: String,
    snapshot_relative_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SuspendedLaunchMod {
    mod_id: String,
    #[serde(default)]
    staged_files: Vec<SuspendedLaunchFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProfileLaunchSuspension {
    version: u32,
    profile_id: String,
    #[serde(default)]
    mods: Vec<SuspendedLaunchMod>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileActionResult {
    profile_id: String,
    name: String,
    removed_mod_records: usize,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileDependencyBootstrapResult {
    profile_id: String,
    installed_dependencies: Vec<String>,
    skipped_dependencies: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModFileHealth {
    installed_mod_id: String,
    mod_name: String,
    checked_files: usize,
    missing_files: Vec<String>,
    #[serde(default)]
    suspended_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileRefreshResult {
    profile: GameProfile,
    detection: GameDetectionResult,
    installed_mods: Vec<InstalledModRecord>,
    mod_file_health: Vec<ModFileHealth>,
    missing_dependencies: Vec<DependencySpec>,
    adopted_native_script_mods: usize,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileGameFolderUpdateResult {
    profile: GameProfile,
    detection: GameDetectionResult,
    installed_mods: Vec<InstalledModRecord>,
    deployed_files: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileExportResult {
    output_path: String,
    profile_name: String,
    exported_mods: usize,
    exported_config_files: usize,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileImportResult {
    profile: GameProfile,
    installed_mods: Vec<InstalledModRecord>,
    deployed_files: Vec<String>,
    config_files_written: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProfileBundleManifest {
    schema_version: u32,
    exported_at: String,
    profile: GameProfile,
    mods: Vec<ProfileBundleMod>,
    config_files: Vec<ProfileBundleConfigFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProfileBundleMod {
    record: InstalledModRecord,
    source_relative_path: String,
    source_is_directory: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProfileBundleConfigFile {
    mod_id: String,
    bundle_path: String,
    target_relative_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default)]
    minimize_to_tray_on_close: bool,
    #[serde(default, skip_serializing)]
    nexus_api_key: String,
    #[serde(default)]
    nexus_api_key_configured: bool,
}

impl AppSettings {
    fn nexus_api_key(&self) -> Option<&str> {
        let trimmed = self.nexus_api_key.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }

    fn nexus_api_ready(&self) -> bool {
        self.nexus_api_key().is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateInfo {
    current_version: String,
    latest_version: Option<String>,
    update_available: bool,
    release_url: Option<String>,
    installer_url: Option<String>,
    installer_name: Option<String>,
    status: String,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NexusNxmInstallResult {
    mod_id: String,
    install_result: InstallResult,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoreFile<T> {
    version: u32,
    items: Vec<T>,
}

#[derive(Debug, Clone)]
struct ProbeEntry {
    relative_path: String,
    name: String,
    is_directory: bool,
    depth: usize,
}

#[derive(Debug, Default)]
struct RoutePreparation {
    expected_mod_folders: Vec<String>,
    created_mod_folders: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct NativeScriptFile {
    absolute_path: PathBuf,
    target_relative_path: String,
    source_relative_path: String,
}

#[derive(Debug, Clone)]
struct ScannedArchive {
    archive_path: String,
    archive_name: String,
    entries: Vec<ArchiveEntry>,
    manifest: Option<ThunderstoreManifest>,
    package_identity: Option<PackageIdentity>,
}

#[tauri::command]
fn get_app_settings(app: AppHandle) -> Result<AppSettings, String> {
    let mut settings = read_app_settings(&store_root(&app)?)?;
    settings.nexus_api_key_configured = settings.nexus_api_ready();
    settings.nexus_api_key.clear();
    Ok(settings)
}

#[tauri::command]
fn update_app_settings(app: AppHandle, input: AppSettings) -> Result<AppSettings, String> {
    let _operation = lock_mutations()?;
    let root = store_root(&app)?;
    if !input.nexus_api_key.trim().is_empty() {
        write_nexus_api_key(Some(input.nexus_api_key.trim()))?;
    }

    let mut saved = input;
    saved.nexus_api_key.clear();
    saved.nexus_api_key_configured = read_nexus_api_key()?.is_some();
    write_app_settings(&root, &saved)?;
    Ok(saved)
}

#[tauri::command]
async fn save_nexus_api_key(app: AppHandle, api_key: String) -> Result<AppSettings, String> {
    tauri::async_runtime::spawn_blocking(move || save_nexus_api_key_impl(app, api_key))
        .await
        .map_err(error_to_string)?
}

fn save_nexus_api_key_impl(app: AppHandle, api_key: String) -> Result<AppSettings, String> {
    let trimmed = api_key.trim().to_string();
    if !trimmed.is_empty() {
        validate_nexus_api_key(&trimmed)?;
    }

    let _operation = lock_mutations()?;
    let root = store_root(&app)?;
    write_nexus_api_key(if trimmed.is_empty() {
        None
    } else {
        Some(&trimmed)
    })?;
    let stored_key = read_nexus_api_key()?;
    if stored_key.as_deref() != (!trimmed.is_empty()).then_some(trimmed.as_str()) {
        return Err(
            "Windows Credential Manager did not retain the Nexus API key. Please try saving it again."
                .to_string(),
        );
    }

    let mut settings = read_app_settings(&root)?;
    settings.nexus_api_key.clear();
    settings.nexus_api_key_configured = stored_key.is_some();
    write_app_settings(&root, &settings)?;
    Ok(settings)
}

#[tauri::command]
async fn check_app_update() -> AppUpdateInfo {
    tauri::async_runtime::spawn_blocking(check_app_update_impl)
        .await
        .unwrap_or_else(|error| AppUpdateInfo {
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            latest_version: None,
            update_available: false,
            release_url: None,
            installer_url: None,
            installer_name: None,
            status: "error".to_string(),
            message: format!("Update check stopped unexpectedly: {error}"),
        })
}

fn check_app_update_impl() -> AppUpdateInfo {
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let request_url = format!("{GITHUB_API_BASE}/{APP_UPDATE_REPOSITORY}/releases/latest");
    let client = match Client::builder().timeout(Duration::from_secs(8)).build() {
        Ok(client) => client,
        Err(error) => {
            return AppUpdateInfo {
                current_version,
                latest_version: None,
                update_available: false,
                release_url: None,
                installer_url: None,
                installer_name: None,
                status: "error".to_string(),
                message: format!("Could not prepare update checker: {error}"),
            };
        }
    };

    let response = match client
        .get(&request_url)
        .header("User-Agent", "UniLoader update checker")
        .send()
    {
        Ok(response) => response,
        Err(error) => {
            return AppUpdateInfo {
                current_version,
                latest_version: None,
                update_available: false,
                release_url: None,
                installer_url: None,
                installer_name: None,
                status: "error".to_string(),
                message: format!("Could not check for updates: {error}"),
            };
        }
    };

    if response.status().as_u16() == 404 {
        return AppUpdateInfo {
            current_version,
            latest_version: None,
            update_available: false,
            release_url: None,
            installer_url: None,
            installer_name: None,
            status: "unavailable".to_string(),
            message: "No GitHub release has been published yet.".to_string(),
        };
    }

    if !response.status().is_success() {
        let status = response.status();
        return AppUpdateInfo {
            current_version,
            latest_version: None,
            update_available: false,
            release_url: None,
            installer_url: None,
            installer_name: None,
            status: "error".to_string(),
            message: format!("GitHub update check failed with HTTP {status}."),
        };
    }

    let release = match response.json::<GithubReleaseResponse>() {
        Ok(release) => release,
        Err(error) => {
            return AppUpdateInfo {
                current_version,
                latest_version: None,
                update_available: false,
                release_url: None,
                installer_url: None,
                installer_name: None,
                status: "error".to_string(),
                message: format!("GitHub release response was not readable: {error}"),
            };
        }
    };

    let latest_version = release.tag_name.trim_start_matches('v').to_string();
    let update_available = is_newer_version(&latest_version, &current_version);
    let installer_asset = select_update_installer_asset(&release);
    AppUpdateInfo {
        current_version: current_version.clone(),
        latest_version: Some(latest_version.clone()),
        update_available,
        release_url: release.html_url.clone(),
        installer_url: installer_asset.map(|asset| asset.browser_download_url.clone()),
        installer_name: installer_asset.map(|asset| asset.name.clone()),
        status: if update_available {
            "available"
        } else {
            "up-to-date"
        }
        .to_string(),
        message: if update_available {
            if installer_asset.is_some() {
                format!(
                    "UniLoader v{latest_version} is available. Click to download the installer."
                )
            } else {
                format!(
                    "UniLoader v{latest_version} is available, but no installer asset was found."
                )
            }
        } else {
            format!("UniLoader v{current_version} is current.")
        },
    }
}

#[tauri::command]
fn list_profiles(app: AppHandle) -> Result<Vec<GameProfile>, String> {
    let root = store_root(&app)?;
    let path = profiles_path(&root);
    let mut store = read_store::<GameProfile>(&path).map_err(error_to_string)?;
    let mut changed = false;

    for profile in &mut store.items {
        let normalized_game_path = normalize_profile_game_path(&profile.game_path);
        if normalized_game_path != profile.game_path {
            profile.game_path = normalized_game_path;
            changed = true;
        }
        if profile
            .steam_app_id
            .as_deref()
            .map(str::trim)
            .is_none_or(str::is_empty)
        {
            if let Some(app_id) = infer_steam_app_id_for_game_path(Path::new(&profile.game_path)) {
                profile.steam_app_id = Some(app_id);
                changed = true;
            }
        }
        if let Some(definition) = profile
            .steam_app_id
            .as_deref()
            .and_then(game_definition_by_steam_app_id)
        {
            if profile.game_id.as_deref() != Some(definition.id.as_str()) {
                profile.game_id = Some(definition.id.clone());
                changed = true;
            }
            if let Some(engine) = definition.engine.as_deref() {
                if profile.engine != engine {
                    profile.engine = engine.to_string();
                    changed = true;
                }
            }
            if profile.loader == "none" {
                if let Some(loader) = definition.bootstrap_runtimes.first() {
                    profile.loader = loader.clone();
                    changed = true;
                }
            }
        }
    }

    if changed {
        write_store(&path, &store).map_err(error_to_string)?;
    }

    let installed_steam_games = scan_steam_games_impl()
        .into_iter()
        .map(|game| {
            (
                game.app_id,
                normalize_filesystem_identity(
                    normalize_profile_game_path(&game.install_dir).trim_end_matches(['/', '\\']),
                ),
            )
        })
        .collect::<HashSet<_>>();
    let verified_profiles = store
        .items
        .into_iter()
        .filter(|profile| {
            profile.steam_app_id.as_deref().is_some_and(|app_id| {
                installed_steam_games.contains(&(
                    app_id.to_string(),
                    normalize_filesystem_identity(
                        normalize_profile_game_path(&profile.game_path)
                            .trim_end_matches(['/', '\\']),
                    ),
                ))
            })
        })
        .collect();
    Ok(verified_profiles)
}

#[tauri::command]
fn profile_folder_exists(app: AppHandle, profile_id: String) -> Result<bool, String> {
    let root = store_root(&app)?;
    let profile = get_profile(&root, &profile_id)?;
    Ok(Path::new(&profile.game_path).is_dir())
}

#[tauri::command]
async fn create_profile(
    _app: AppHandle,
    _input: CreateProfileInput,
) -> Result<GameProfile, String> {
    Err("Only installed Steam games can be added as UniLoader profiles.".to_string())
}

fn create_profile_in_store(root: &Path, input: CreateProfileInput) -> Result<GameProfile, String> {
    let trimmed_name = input.name.trim().to_string();
    if trimmed_name.is_empty() {
        return Err("Profile name is required.".to_string());
    }

    let path = profiles_path(root);
    let mut store = read_store::<GameProfile>(&path).map_err(error_to_string)?;
    let now = now_string();
    let profile = GameProfile {
        id: Uuid::new_v4().to_string(),
        name: trimmed_name,
        game_path: normalize_profile_game_path(&input.game_path),
        game_id: input.game_id,
        steam_app_id: input.steam_app_id,
        engine: input.engine,
        loader: input.loader,
        created_at: now.clone(),
        updated_at: now,
    };

    store.items.push(profile.clone());
    write_store(&path, &store).map_err(error_to_string)?;
    fs::create_dir_all(profile_dir(root, &profile.id)).map_err(error_to_string)?;
    Ok(profile)
}

#[tauri::command]
async fn scan_steam_games() -> Result<Vec<SteamGameRecord>, String> {
    tauri::async_runtime::spawn_blocking(scan_steam_games_impl)
        .await
        .map_err(error_to_string)
}

#[tauri::command]
async fn create_steam_profile(
    app: AppHandle,
    game: SteamGameRecord,
) -> Result<GameProfile, String> {
    tauri::async_runtime::spawn_blocking(move || create_steam_profile_sync(app, game))
        .await
        .map_err(error_to_string)?
}

fn create_steam_profile_sync(app: AppHandle, game: SteamGameRecord) -> Result<GameProfile, String> {
    let _operation = lock_mutations()?;
    let root = store_root(&app)?;
    let game = verify_installed_steam_game(&game)?;
    let existing_profiles =
        read_store::<GameProfile>(&profiles_path(&root)).map_err(error_to_string)?;
    if existing_profiles.items.iter().any(|profile| {
        profile.steam_app_id.as_deref() == Some(game.app_id.as_str())
            || normalize_filesystem_identity(&profile.game_path)
                == normalize_filesystem_identity(&game.install_dir)
    }) {
        return Err(format!("{} already has a UniLoader profile.", game.name));
    }
    let detection =
        detect_game_setup_with_steam_app_id(Path::new(&game.install_dir), Some(&game.app_id))?;
    let mut profile = create_profile_in_store(
        &root,
        CreateProfileInput {
            name: game.name,
            game_path: game.install_dir,
            game_id: detection.game_id,
            steam_app_id: Some(game.app_id),
            engine: detection.engine,
            loader: detection.loader,
        },
    )?;
    if enrich_profile_with_provider_runtime(&mut profile).is_some() {
        persist_profile(&root, &profile)?;
    }
    let _ = install_profile_bootstrap_dependencies(&root, &profile);
    let _ = ensure_profile_route_knowledge(&root, &profile, true);
    Ok(profile)
}

fn verify_installed_steam_game(requested: &SteamGameRecord) -> Result<SteamGameRecord, String> {
    let requested_path = normalize_filesystem_identity(
        normalize_profile_game_path(&requested.install_dir).trim_end_matches(['/', '\\']),
    );

    scan_steam_games_impl()
        .into_iter()
        .find(|installed| {
            installed.app_id == requested.app_id
                && normalize_filesystem_identity(
                    normalize_profile_game_path(&installed.install_dir)
                        .trim_end_matches(['/', '\\']),
                ) == requested_path
        })
        .ok_or_else(|| {
            "Steam could not verify this installed game. Rescan your Steam library and select it again."
                .to_string()
        })
}

fn ensure_verified_steam_profile(profile: &GameProfile) -> Result<(), String> {
    let app_id = profile
        .steam_app_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Only verified Steam profiles can install mods.".to_string())?;

    verify_installed_steam_game(&SteamGameRecord {
        app_id: app_id.to_string(),
        name: profile.name.clone(),
        install_dir: profile.game_path.clone(),
        library_path: String::new(),
    })
    .map(|_| ())
}

#[tauri::command]
fn launch_profile_game(
    app: AppHandle,
    profile_id: String,
    mods_enabled: bool,
) -> Result<(), String> {
    let _operation = lock_mutations()?;
    let root = store_root(&app)?;
    let profile = get_profile(&root, &profile_id)?;
    let steam_app_id = profile
        .steam_app_id
        .as_deref()
        .map(str::trim)
        .filter(|app_id| !app_id.is_empty())
        .ok_or_else(|| {
            "This profile does not have a Steam App ID yet. Add it through Steam scan to launch it from UniLoader.".to_string()
        })?;

    prepare_profile_mod_launch(&root, &profile, mods_enabled)?;
    open_url_in_shell(&format!("steam://run/{steam_app_id}"))
}

#[tauri::command]
fn profile_game_running(app: AppHandle, profile_id: String) -> Result<bool, String> {
    let root = store_root(&app)?;
    let profile = get_profile(&root, &profile_id)?;
    if profile
        .steam_app_id
        .as_deref()
        .map(str::trim)
        .is_none_or(str::is_empty)
    {
        return Ok(false);
    }

    game_process_running(Path::new(&profile.game_path))
}

#[cfg(target_os = "windows")]
fn game_process_running(game_path: &Path) -> Result<bool, String> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    let game_root = fs::canonicalize(game_path).unwrap_or_else(|_| game_path.to_path_buf());
    if !game_root.is_dir() {
        return Ok(false);
    }

    // SAFETY: Toolhelp returns an owned snapshot handle that is closed before this function exits.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(format!(
            "Could not inspect running games: {}",
            io::Error::last_os_error()
        ));
    }

    let mut process = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    // SAFETY: `process` is correctly sized and remains valid throughout enumeration.
    let mut has_process = unsafe { Process32FirstW(snapshot, &mut process) } != 0;
    let mut found = false;

    while has_process {
        if process.th32ProcessID != 0
            && process.th32ProcessID != std::process::id()
            && process_executable_path(process.th32ProcessID)
                .is_some_and(|executable| path_is_within_directory(&executable, &game_root))
        {
            found = true;
            break;
        }

        // SAFETY: `process` remains a valid, correctly sized output buffer for the snapshot.
        has_process = unsafe { Process32NextW(snapshot, &mut process) } != 0;
    }

    // SAFETY: `snapshot` is a valid owned Toolhelp handle and is closed exactly once.
    unsafe {
        CloseHandle(snapshot);
    }
    Ok(found)
}

#[cfg(target_os = "windows")]
fn process_executable_path(process_id: u32) -> Option<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    // SAFETY: OpenProcess is called with query-only access for the supplied process ID.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if handle.is_null() {
        return None;
    }

    let mut buffer = vec![0u16; 32_768];
    let mut length = buffer.len() as u32;
    // SAFETY: `buffer` is writable for `length` UTF-16 units and the handle is valid.
    let queried =
        unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut length) } != 0;
    // SAFETY: `handle` was returned by OpenProcess and is closed exactly once.
    unsafe {
        CloseHandle(handle);
    }

    queried.then(|| PathBuf::from(OsString::from_wide(&buffer[..length as usize])))
}

#[cfg(not(target_os = "windows"))]
fn game_process_running(_game_path: &Path) -> Result<bool, String> {
    Ok(false)
}

fn path_is_within_directory(path: &Path, directory: &Path) -> bool {
    let normalized_path = normalize_process_path(path);
    let normalized_directory = normalize_process_path(directory);
    let directory_prefix = format!("{}/", normalized_directory.trim_end_matches('/'));
    normalized_path.starts_with(&directory_prefix)
}

fn normalize_process_path(path: &Path) -> String {
    let normalized = normalize_filesystem_identity(path.to_string_lossy().as_ref());
    normalized
        .strip_prefix("//?/")
        .unwrap_or(&normalized)
        .to_string()
}

fn scan_steam_games_impl() -> Vec<SteamGameRecord> {
    let mut games = Vec::new();
    let mut seen_app_ids = HashSet::new();

    for library_path in steam_library_roots() {
        let steamapps_path = library_path.join("steamapps");
        let Ok(entries) = fs::read_dir(&steamapps_path) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };

            if !file_name.starts_with("appmanifest_") || !file_name.ends_with(".acf") {
                continue;
            }

            let Some(game) = parse_steam_app_manifest(&path, &library_path) else {
                continue;
            };

            if is_user_steam_game(&game) && seen_app_ids.insert(game.app_id.clone()) {
                games.push(game);
            }
        }
    }

    games.sort_by(|first, second| {
        first
            .name
            .to_lowercase()
            .cmp(&second.name.to_lowercase())
            .then_with(|| first.app_id.cmp(&second.app_id))
    });
    games
}

fn is_user_steam_game(game: &SteamGameRecord) -> bool {
    let lower_name = game.name.to_lowercase();
    Path::new(&game.install_dir).is_dir()
        && game.app_id != "228980"
        && !lower_name.contains("steamworks common redistributables")
        && !lower_name.starts_with("steam linux runtime")
        && !lower_name.contains("proton ")
        && !lower_name.ends_with(" dedicated server")
}

fn steam_library_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut seen = HashSet::new();

    for steam_root in steam_install_roots() {
        push_unique_path(&mut roots, &mut seen, steam_root.clone());

        let library_file = steam_root.join("steamapps").join("libraryfolders.vdf");
        let Ok(content) = fs::read_to_string(&library_file) else {
            continue;
        };

        for library_path in parse_steam_library_paths(&content) {
            push_unique_path(&mut roots, &mut seen, library_path);
        }
    }

    roots
}

fn steam_install_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut seen = HashSet::new();

    for root in steam_registry_roots() {
        push_unique_path(&mut roots, &mut seen, root);
    }

    if let Ok(program_files_x86) = std::env::var("ProgramFiles(x86)") {
        push_unique_path(
            &mut roots,
            &mut seen,
            PathBuf::from(program_files_x86).join("Steam"),
        );
    }

    if let Ok(program_files) = std::env::var("ProgramFiles") {
        push_unique_path(
            &mut roots,
            &mut seen,
            PathBuf::from(program_files).join("Steam"),
        );
    }

    push_unique_path(
        &mut roots,
        &mut seen,
        PathBuf::from(r"C:\Program Files (x86)\Steam"),
    );
    roots
        .into_iter()
        .filter(|root| root.join("steamapps").is_dir())
        .collect()
}

#[cfg(windows)]
fn steam_registry_roots() -> Vec<PathBuf> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let mut roots = Vec::new();

    for (hive, key_path, value_name) in [
        (
            RegKey::predef(HKEY_CURRENT_USER),
            r"Software\Valve\Steam",
            "SteamPath",
        ),
        (
            RegKey::predef(HKEY_LOCAL_MACHINE),
            r"SOFTWARE\WOW6432Node\Valve\Steam",
            "InstallPath",
        ),
        (
            RegKey::predef(HKEY_LOCAL_MACHINE),
            r"SOFTWARE\Valve\Steam",
            "InstallPath",
        ),
    ] {
        if let Ok(key) = hive.open_subkey(key_path) {
            if let Ok(path) = key.get_value::<String, _>(value_name) {
                let path = path.trim();
                if !path.is_empty() {
                    roots.push(PathBuf::from(path));
                }
            }
        }
    }

    roots
}

#[cfg(not(windows))]
fn steam_registry_roots() -> Vec<PathBuf> {
    Vec::new()
}

fn parse_steam_library_paths(content: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    for line in content.lines() {
        let tokens = quoted_values(line);
        if tokens.len() < 2 {
            continue;
        }

        let key = tokens[0].trim();
        let value = tokens[1].trim();
        if key.eq_ignore_ascii_case("path")
            || (key.chars().all(|ch| ch.is_ascii_digit()) && looks_like_path(value))
        {
            paths.push(PathBuf::from(value));
        }
    }

    paths
}

fn parse_steam_app_manifest(path: &Path, library_path: &Path) -> Option<SteamGameRecord> {
    let content = fs::read_to_string(path).ok()?;
    let mut values: HashMap<String, String> = HashMap::new();

    for line in content.lines() {
        let tokens = quoted_values(line);
        if tokens.len() >= 2 {
            values.insert(tokens[0].to_lowercase(), tokens[1].clone());
        }
    }

    let app_id = values.get("appid")?.trim().to_string();
    let name = values.get("name")?.trim().to_string();
    let install_dir_name = values
        .get("installdir")
        .map(String::as_str)
        .unwrap_or(name.as_str())
        .trim();
    let install_dir = library_path
        .join("steamapps")
        .join("common")
        .join(install_dir_name);

    Some(SteamGameRecord {
        app_id,
        name,
        install_dir: install_dir.to_string_lossy().to_string(),
        library_path: library_path.to_string_lossy().to_string(),
    })
}

fn infer_steam_app_id_for_game_path(game_path: &Path) -> Option<String> {
    let common_path = game_path.parent()?;
    if !common_path
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("common"))
    {
        return None;
    }

    let steamapps_path = common_path.parent()?;
    if !steamapps_path
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("steamapps"))
    {
        return None;
    }

    let library_path = steamapps_path.parent()?;
    let target_identity = normalize_filesystem_identity(&game_path.to_string_lossy());
    let entries = fs::read_dir(steamapps_path).ok()?;

    for entry in entries.flatten() {
        let manifest_path = entry.path();
        let Some(file_name) = manifest_path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !file_name.starts_with("appmanifest_") || !file_name.ends_with(".acf") {
            continue;
        }

        let Some(game) = parse_steam_app_manifest(&manifest_path, library_path) else {
            continue;
        };
        if normalize_filesystem_identity(&game.install_dir) == target_identity {
            return Some(game.app_id);
        }
    }

    None
}

fn quoted_values(line: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '"' {
            continue;
        }

        let mut value = String::new();
        let mut escaped = false;

        for next_ch in chars.by_ref() {
            if escaped {
                value.push(next_ch);
                escaped = false;
                continue;
            }

            if next_ch == '\\' {
                escaped = true;
                continue;
            }

            if next_ch == '"' {
                break;
            }

            value.push(next_ch);
        }

        values.push(value);
    }

    values
}

fn looks_like_path(value: &str) -> bool {
    value.contains(":\\") || value.contains(":/") || value.contains('\\') || value.contains('/')
}

fn push_unique_path(paths: &mut Vec<PathBuf>, seen: &mut HashSet<String>, path: PathBuf) {
    let key = path.to_string_lossy().replace('/', "\\").to_lowercase();
    if seen.insert(key) {
        paths.push(path);
    }
}

#[tauri::command]
fn rename_profile(app: AppHandle, profile_id: String, name: String) -> Result<GameProfile, String> {
    let _operation = lock_mutations()?;
    let trimmed_name = name.trim().to_string();
    if trimmed_name.is_empty() {
        return Err("Profile name is required.".to_string());
    }

    let root = store_root(&app)?;
    let path = profiles_path(&root);
    let mut store = read_store::<GameProfile>(&path).map_err(error_to_string)?;
    let profile = store
        .items
        .iter_mut()
        .find(|profile| profile.id == profile_id)
        .ok_or_else(|| format!("Profile not found: {}", profile_id))?;

    profile.name = trimmed_name;
    profile.updated_at = now_string();
    let updated_profile = profile.clone();
    write_store(&path, &store).map_err(error_to_string)?;
    Ok(updated_profile)
}

#[tauri::command]
fn remove_profile(app: AppHandle, profile_id: String) -> Result<ProfileActionResult, String> {
    let _operation = lock_mutations()?;
    let root = store_root(&app)?;
    let profiles_file = profiles_path(&root);
    let mut profiles = read_store::<GameProfile>(&profiles_file).map_err(error_to_string)?;
    let profile_index = profiles
        .items
        .iter()
        .position(|profile| profile.id == profile_id)
        .ok_or_else(|| format!("Profile not found: {}", profile_id))?;
    let profile = profiles.items[profile_index].clone();

    let installed_mods_file = installed_mods_path(&root);
    let mut installed_mods =
        read_store::<InstalledModRecord>(&installed_mods_file).map_err(error_to_string)?;
    let profile_mods = installed_mods
        .items
        .iter()
        .filter(|record| record.profile_id == profile.id)
        .cloned()
        .collect::<Vec<_>>();
    let mut deactivated: Vec<InstalledModRecord> = Vec::new();
    for record in profile_mods
        .iter()
        .filter(|record| record.enabled && record.runtime_id.is_none())
    {
        if let Err(error) = deactivate_mod_files(&root, &profile, record) {
            for previous in deactivated.iter().rev() {
                let mut previous = previous.clone();
                if let Some(plan) = previous.plan.clone() {
                    let install_id = previous.id.clone();
                    let archive_path = previous.archive_path.clone();
                    let _ = deploy_mod_files(
                        &root,
                        &profile,
                        &install_id,
                        &archive_path,
                        &plan,
                        &mut previous,
                    );
                }
            }
            return Err(format!(
                "Profile removal was cancelled because its mod files could not be safely removed: {error}"
            ));
        }
        deactivated.push(record.clone());
    }

    profiles.items.remove(profile_index);
    let before_count = installed_mods.items.len();
    installed_mods
        .items
        .retain(|record| record.profile_id != profile.id);
    let removed_mod_records = before_count.saturating_sub(installed_mods.items.len());

    let previous_installed_mods =
        read_store::<InstalledModRecord>(&installed_mods_file).map_err(error_to_string)?;
    if let Err(error) = write_store(&installed_mods_file, &installed_mods) {
        restore_deactivated_mods(&root, &profile, &deactivated);
        return Err(error_to_string(error));
    }
    if let Err(error) = write_store(&profiles_file, &profiles) {
        let _ = write_store(&installed_mods_file, &previous_installed_mods);
        restore_deactivated_mods(&root, &profile, &deactivated);
        return Err(error_to_string(error));
    }

    let mut warnings = Vec::new();
    if let Err(error) = remove_pending_nexus_downloads_for_profile(&root, &profile.id) {
        warnings.push(format!(
            "Profile was removed, but UniLoader could not clear its pending Nexus downloads: {error}"
        ));
    }
    let profile_data_dir = profile_dir(&root, &profile.id);
    if profile_data_dir.exists() {
        if let Err(error) = fs::remove_dir_all(&profile_data_dir) {
            warnings.push(format!(
                "Profile was removed, but UniLoader could not remove its local data folder: {}.",
                error
            ));
        }
    }

    Ok(ProfileActionResult {
        profile_id: profile.id,
        name: profile.name,
        removed_mod_records,
        warnings,
    })
}

#[tauri::command]
async fn refresh_profile(
    app: AppHandle,
    profile_id: String,
) -> Result<ProfileRefreshResult, String> {
    tauri::async_runtime::spawn_blocking(move || refresh_profile_sync(app, profile_id))
        .await
        .map_err(error_to_string)?
}

fn refresh_profile_sync(
    app: AppHandle,
    profile_id: String,
) -> Result<ProfileRefreshResult, String> {
    let _operation = lock_mutations()?;
    let root = store_root(&app)?;
    let profiles_file = profiles_path(&root);
    let mut profiles = read_store::<GameProfile>(&profiles_file).map_err(error_to_string)?;
    let profile_index = profiles
        .items
        .iter()
        .position(|profile| profile.id == profile_id)
        .ok_or_else(|| format!("Profile not found: {}", profile_id))?;
    let mut profile = profiles.items[profile_index].clone();
    ensure_verified_steam_profile(&profile)?;
    let mut detection = detect_game_setup_with_steam_app_id(
        Path::new(&profile.game_path),
        profile.steam_app_id.as_deref(),
    )?;

    profile.game_id = detection.game_id.clone().or(profile.game_id);
    if detection.engine != "unknown" || profile.engine == "unknown" {
        profile.engine = detection.engine.clone();
    }
    if detection.loader != "none" || runtime_definition_by_id(&profile.loader).is_none() {
        profile.loader = detection.loader.clone();
    }
    let runtime_inference = enrich_profile_with_provider_runtime(&mut profile);
    profile.updated_at = now_string();
    profiles.items[profile_index] = profile.clone();

    let bootstrap_warnings = install_profile_bootstrap_dependencies(&root, &profile);
    let route_outcome = ensure_profile_route_knowledge(&root, &profile, false);
    for route in &route_outcome.expected_routes {
        push_unique_route(&mut detection.expected_mod_folders, route);
    }
    for route in &route_outcome.created_routes {
        push_unique_route(&mut detection.created_mod_folders, route);
    }
    apply_profile_identity_to_detection(&profile, &mut detection, runtime_inference.as_ref());

    let discovered_config_files = discover_profile_config_files(&profile);
    let installed_mods_file = installed_mods_path(&root);
    let mut installed_store =
        read_store::<InstalledModRecord>(&installed_mods_file).map_err(error_to_string)?;
    let original_installed_store = installed_store.clone();
    let adopted_native_script_mods =
        adopt_existing_native_script_mods(&root, &profile, &mut installed_store)?;
    ensure_visible_runtime_records(&root, &profile, &mut installed_store)?;
    let settings = read_app_settings(&root).ok();
    backfill_installed_mod_artwork(&root, &profile, settings.as_ref(), &mut installed_store);
    let launch_suspension = read_profile_launch_suspension(&root, &profile.id)?;
    let mut mod_file_health = Vec::new();
    let mut missing_dependencies = Vec::new();
    let mut dependency_keys = HashSet::new();

    for dependency in profile_bootstrap_dependencies(&profile) {
        let dependency = refresh_dependency_status(&root, &profile, &dependency);
        push_missing_dependency(&mut missing_dependencies, &mut dependency_keys, dependency);
    }

    for record in installed_store
        .items
        .iter_mut()
        .filter(|record| record.profile_id == profile_id)
    {
        record.display_name = record
            .display_name
            .as_deref()
            .map(humanize_mod_display_name);
        record.config_files =
            resolved_config_files_for_record(&profile, record, &discovered_config_files);
        record.dependencies = record
            .dependencies
            .iter()
            .map(|dependency| refresh_dependency_status(&root, &profile, dependency))
            .collect();

        for dependency in &record.dependencies {
            push_missing_dependency(
                &mut missing_dependencies,
                &mut dependency_keys,
                dependency.clone(),
            );
        }

        mod_file_health.push(mod_file_health_for_record(record, &launch_suspension));
    }

    write_store(&installed_mods_file, &installed_store).map_err(error_to_string)?;
    if let Err(error) = write_store(&profiles_file, &profiles) {
        let _ = write_store(&installed_mods_file, &original_installed_store);
        return Err(error_to_string(error));
    }

    let installed_mods = installed_store
        .items
        .iter()
        .filter(|record| record.profile_id == profile_id)
        .cloned()
        .collect::<Vec<_>>();
    let mut warnings =
        profile_refresh_warnings(&detection, &mod_file_health, &missing_dependencies);
    warnings.extend(bootstrap_warnings);
    warnings.extend(route_outcome.warnings);

    Ok(ProfileRefreshResult {
        profile,
        detection,
        installed_mods,
        mod_file_health,
        missing_dependencies,
        adopted_native_script_mods,
        warnings,
    })
}

#[tauri::command]
async fn update_profile_game_folder(
    _app: AppHandle,
    _profile_id: String,
    _game_path: String,
) -> Result<ProfileGameFolderUpdateResult, String> {
    Err(
        "Steam profile folders are managed by Steam. Rescan the Steam library after moving a game."
            .to_string(),
    )
}

#[tauri::command]
async fn bootstrap_profile_dependencies(
    app: AppHandle,
    profile_id: String,
) -> Result<ProfileDependencyBootstrapResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        bootstrap_profile_dependencies_sync(app, profile_id)
    })
    .await
    .map_err(error_to_string)?
}

fn bootstrap_profile_dependencies_sync(
    app: AppHandle,
    profile_id: String,
) -> Result<ProfileDependencyBootstrapResult, String> {
    let _operation = lock_mutations()?;
    let root = store_root(&app)?;
    let mut profile = get_profile(&root, &profile_id)?;
    ensure_verified_steam_profile(&profile)?;
    repair_profile_identity_from_steam_app_id(&root, &mut profile)?;
    let mut visited_dependencies = HashSet::new();
    let mut installed_dependencies = Vec::new();
    let mut skipped_dependencies = Vec::new();
    let mut warnings = Vec::new();
    let mut seen_dependencies = HashSet::new();

    for dependency in profile_dependency_candidates(&root, &profile)? {
        if !seen_dependencies.insert(dependency_key(&dependency)) {
            continue;
        }
        if dependency.status == "already-installed" {
            skipped_dependencies.push(dependency.name.clone());
            continue;
        }

        let install_result = install_dependency_by_provider(
            &root,
            &profile,
            &dependency,
            &mut visited_dependencies,
            0,
        );

        match install_result {
            Ok(mut dependency_warnings) => {
                let verified = refresh_dependency_status(&root, &profile, &dependency);
                if verified.status == "already-installed" {
                    installed_dependencies.push(dependency.name.clone());
                    warnings.append(&mut dependency_warnings);
                } else {
                    warnings.push(format!(
                        "{} was downloaded, but its required runtime files were not detected in the game folder after installation.",
                        dependency.name
                    ));
                }
            }
            Err(error) => warnings.push(format!(
                "Could not install {} automatically: {}",
                dependency.name, error
            )),
        }
    }

    Ok(ProfileDependencyBootstrapResult {
        profile_id,
        installed_dependencies,
        skipped_dependencies,
        warnings,
    })
}

#[tauri::command]
async fn export_profile_bundle(
    app: AppHandle,
    profile_id: String,
    output_path: String,
) -> Result<ProfileExportResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = lock_mutations()?;
        let root = store_root(&app)?;
        export_profile_bundle_impl(&root, &profile_id, Path::new(&output_path))
    })
    .await
    .map_err(error_to_string)?
}

#[tauri::command]
async fn import_profile_bundle(
    app: AppHandle,
    bundle_path: String,
) -> Result<ProfileImportResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = lock_mutations()?;
        let root = store_root(&app)?;
        let game = resolve_profile_bundle_steam_game(
            Path::new(&bundle_path),
            &scan_steam_games_impl(),
        )?;
        let profiles = read_store::<GameProfile>(&profiles_path(&root)).map_err(error_to_string)?;
        if profiles.items.iter().any(|profile| {
            profile.steam_app_id.as_deref() == Some(game.app_id.as_str())
                || normalize_filesystem_identity(&profile.game_path)
                    == normalize_filesystem_identity(&game.install_dir)
        }) {
            return Err(format!(
                "{} already has a UniLoader profile. Remove the existing profile before importing a replacement bundle.",
                game.name
            ));
        }

        import_profile_bundle_impl(&root, Path::new(&bundle_path), Path::new(&game.install_dir))
    })
    .await
    .map_err(error_to_string)?
}

fn profile_dependency_candidates(
    root: &Path,
    profile: &GameProfile,
) -> Result<Vec<DependencySpec>, String> {
    let mut dependencies = profile_bootstrap_dependencies(profile)
        .into_iter()
        .map(|dependency| refresh_dependency_status(root, profile, &dependency))
        .collect::<Vec<_>>();

    let installed_store =
        read_store::<InstalledModRecord>(&installed_mods_path(root)).map_err(error_to_string)?;
    for record in installed_store
        .items
        .iter()
        .filter(|record| record.profile_id == profile.id)
    {
        dependencies.extend(
            record
                .dependencies
                .iter()
                .map(|dependency| refresh_dependency_status(root, profile, dependency)),
        );
    }

    Ok(dependencies)
}

#[allow(dead_code)]
fn update_profile_game_folder_impl(
    root: &Path,
    profile_id: &str,
    game_path: &str,
) -> Result<ProfileGameFolderUpdateResult, String> {
    let normalized_game_path = normalize_profile_game_path(game_path);
    let steam_app_id = infer_steam_app_id_for_game_path(Path::new(&normalized_game_path));
    let mut detection = detect_game_setup_with_steam_app_id(
        Path::new(&normalized_game_path),
        steam_app_id.as_deref(),
    )?;
    let profiles_file = profiles_path(root);
    let mut profiles = read_store::<GameProfile>(&profiles_file).map_err(error_to_string)?;
    let profile_index = profiles
        .items
        .iter()
        .position(|profile| profile.id == profile_id)
        .ok_or_else(|| format!("Profile not found: {}", profile_id))?;

    let old_profile = profiles.items[profile_index].clone();
    if normalize_filesystem_identity(&old_profile.game_path)
        == normalize_filesystem_identity(&normalized_game_path)
    {
        return Ok(ProfileGameFolderUpdateResult {
            profile: old_profile,
            detection,
            installed_mods: read_store::<InstalledModRecord>(&installed_mods_path(root))
                .map_err(error_to_string)?
                .items
                .into_iter()
                .filter(|record| record.profile_id == profile_id)
                .collect(),
            deployed_files: Vec::new(),
            warnings: vec!["The selected game folder is already active.".to_string()],
        });
    }

    let mut profile = old_profile.clone();
    profile.game_path = normalized_game_path.clone();
    profile.game_id = detection.game_id.clone();
    profile.steam_app_id = steam_app_id;
    profile.engine = detection.engine.clone();
    profile.loader = detection.loader.clone();
    let runtime_inference = enrich_profile_with_provider_runtime(&mut profile);
    profile.updated_at = now_string();

    let installed_mods_file = installed_mods_path(root);
    let original_mod_store =
        read_store::<InstalledModRecord>(&installed_mods_file).map_err(error_to_string)?;
    let mut next_mod_store = StoreFile {
        version: original_mod_store.version,
        items: original_mod_store.items.clone(),
    };
    next_mod_store
        .items
        .retain(|record| record.profile_id != profile_id || record.runtime_id.is_none());
    validate_profile_migration_plans(&profile, &next_mod_store.items)?;

    let enabled_originals = original_mod_store
        .items
        .iter()
        .filter(|record| {
            record.profile_id == profile_id && record.enabled && record.runtime_id.is_none()
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut deactivated = Vec::new();
    for record in &enabled_originals {
        if let Err(error) = deactivate_mod_files(root, &old_profile, record) {
            restore_deactivated_mods(root, &old_profile, &deactivated);
            return Err(format!("Game-folder migration was cancelled: {error}"));
        }
        deactivated.push(record.clone());
        let _ = fs::remove_dir_all(profile_backup_dir(root, profile_id, &record.id));
    }

    let mut migrated_records = Vec::new();
    let migration = (|| -> Result<Vec<String>, String> {
        let mut deployed_files = Vec::new();
        for record in next_mod_store
            .items
            .iter_mut()
            .filter(|record| record.profile_id == profile_id && record.enabled)
        {
            let plan = record.plan.clone().ok_or_else(|| {
                format!(
                    "{} cannot be migrated because its original install plan is unavailable.",
                    display_record_name(record)
                )
            })?;
            if let Some(reason) = incompatible_install_plan_reason(&profile, &plan) {
                return Err(format!("{}: {reason}", display_record_name(record)));
            }
            record.files_written.clear();
            record.backups_written.clear();
            record.written_file_hashes.clear();
            let install_id = record.id.clone();
            let archive_path = record.archive_path.clone();
            let files =
                deploy_mod_files(root, &profile, &install_id, &archive_path, &plan, record)?;
            record.config_files = config_files_from_paths(&files);
            record.last_status = "installed".to_string();
            deployed_files.extend(files);
            migrated_records.push(record.clone());
            write_receipt(root, &profile, record)?;
        }
        Ok(deployed_files)
    })();

    let deployed_files = match migration {
        Ok(files) => files,
        Err(error) => {
            rollback_profile_migration(
                root,
                &old_profile,
                &profile,
                &migrated_records,
                &deactivated,
            );
            return Err(format!("Game-folder migration was rolled back: {error}"));
        }
    };

    profiles.items[profile_index] = profile.clone();
    if let Err(error) = write_store(&installed_mods_file, &next_mod_store) {
        rollback_profile_migration(
            root,
            &old_profile,
            &profile,
            &migrated_records,
            &deactivated,
        );
        return Err(error_to_string(error));
    }
    if let Err(error) = write_store(&profiles_file, &profiles) {
        let _ = write_store(&installed_mods_file, &original_mod_store);
        rollback_profile_migration(
            root,
            &old_profile,
            &profile,
            &migrated_records,
            &deactivated,
        );
        return Err(error_to_string(error));
    }

    let warnings = install_profile_bootstrap_dependencies(root, &profile);
    apply_profile_identity_to_detection(&profile, &mut detection, runtime_inference.as_ref());
    let installed_mods = read_store::<InstalledModRecord>(&installed_mods_file)
        .map_err(error_to_string)?
        .items
        .into_iter()
        .filter(|record| record.profile_id == profile_id)
        .collect();

    Ok(ProfileGameFolderUpdateResult {
        profile,
        detection,
        installed_mods,
        deployed_files,
        warnings,
    })
}

fn export_profile_bundle_impl(
    root: &Path,
    profile_id: &str,
    output_path: &Path,
) -> Result<ProfileExportResult, String> {
    let profile = get_profile(root, profile_id)?;
    let discovered_config_files = discover_profile_config_files(&profile);
    let installed_mods = read_store::<InstalledModRecord>(&installed_mods_path(root))
        .map_err(error_to_string)?
        .items
        .into_iter()
        .filter(|record| record.profile_id == profile_id && record.runtime_id.is_none())
        .collect::<Vec<_>>();

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(error_to_string)?;
    }

    let output = File::create(output_path).map_err(error_to_string)?;
    let mut zip = ZipWriter::new(output);
    let mut manifest = ProfileBundleManifest {
        schema_version: 1,
        exported_at: now_string(),
        profile: profile.clone(),
        mods: Vec::new(),
        config_files: Vec::new(),
    };
    let mut warnings = Vec::new();
    let mut seen_config_bundle_paths = HashSet::new();

    for mut record in installed_mods {
        let source_path = Path::new(&record.archive_path);
        if !source_path.exists() {
            warnings.push(format!(
                "Skipped {} because its managed package source is missing.",
                display_record_name(&record)
            ));
            continue;
        }

        let (source_relative_path, source_is_directory) = if source_path.is_dir() {
            ("source".to_string(), true)
        } else {
            let file_name = source_path
                .file_name()
                .and_then(|name| name.to_str())
                .map(sanitize_file_segment)
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "source.zip".to_string());
            (file_name, false)
        };
        let package_entry = format!(
            "packages/{}/{}",
            sanitize_file_segment(&record.id),
            source_relative_path
        );

        if source_is_directory {
            add_directory_to_zip(&mut zip, source_path, &package_entry)?;
        } else {
            add_file_to_zip(&mut zip, source_path, &package_entry)?;
        }

        record.config_files =
            resolved_config_files_for_record(&profile, &record, &discovered_config_files);
        for config_file in &record.config_files {
            let config_path = Path::new(config_file);
            if !config_path.is_file() {
                continue;
            }
            let Ok(relative_path) = config_path.strip_prefix(Path::new(&profile.game_path)) else {
                warnings.push(format!(
                    "Skipped config outside game folder: {}",
                    config_path.to_string_lossy()
                ));
                continue;
            };
            let relative_config_path = to_portable_path(relative_path);
            let bundle_path = format!(
                "configs/{}/{}",
                sanitize_file_segment(&record.id),
                relative_config_path
            );
            if seen_config_bundle_paths.insert(bundle_path.clone()) {
                add_file_to_zip(&mut zip, config_path, &bundle_path)?;
                manifest.config_files.push(ProfileBundleConfigFile {
                    mod_id: record.id.clone(),
                    bundle_path,
                    target_relative_path: relative_config_path,
                });
            }
        }

        manifest.mods.push(ProfileBundleMod {
            record,
            source_relative_path,
            source_is_directory,
        });
    }

    let manifest_content = serde_json::to_vec_pretty(&manifest).map_err(error_to_string)?;
    add_bytes_to_zip(&mut zip, "manifest.json", &manifest_content)?;
    zip.finish().map_err(error_to_string)?;

    Ok(ProfileExportResult {
        output_path: output_path.to_string_lossy().to_string(),
        profile_name: profile.name,
        exported_mods: manifest.mods.len(),
        exported_config_files: manifest.config_files.len(),
        warnings,
    })
}

fn resolve_profile_bundle_steam_game(
    bundle_path: &Path,
    installed_games: &[SteamGameRecord],
) -> Result<SteamGameRecord, String> {
    if !bundle_path.is_file() {
        return Err(format!(
            "Profile bundle does not exist: {}",
            bundle_path.to_string_lossy()
        ));
    }

    let bundle_file = File::open(bundle_path).map_err(error_to_string)?;
    let mut zip = ZipArchive::new(bundle_file).map_err(error_to_string)?;
    validate_zip_archive_safety(&mut zip)?;
    let manifest = read_bundle_manifest(&mut zip)?;
    if manifest.schema_version != 1 {
        return Err(format!(
            "Unsupported profile bundle version: {}",
            manifest.schema_version
        ));
    }
    let steam_app_id = manifest
        .profile
        .steam_app_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "This profile bundle predates Steam-only profiles and cannot be verified safely."
                .to_string()
        })?;

    installed_games
        .iter()
        .find(|game| game.app_id == steam_app_id && Path::new(&game.install_dir).is_dir())
        .cloned()
        .ok_or_else(|| {
            format!(
                "The matching Steam game is not installed. Install Steam App {steam_app_id}, then import this profile again."
            )
        })
}

#[allow(dead_code)]
fn import_profile_bundle_impl(
    root: &Path,
    bundle_path: &Path,
    game_path: &Path,
) -> Result<ProfileImportResult, String> {
    let profiles_file = profiles_path(root);
    let installed_mods_file = installed_mods_path(root);
    let profiles_before = read_store::<GameProfile>(&profiles_file).map_err(error_to_string)?;
    let mods_before =
        read_store::<InstalledModRecord>(&installed_mods_file).map_err(error_to_string)?;
    let existing_profile_ids = profiles_before
        .items
        .iter()
        .map(|profile| profile.id.clone())
        .collect::<HashSet<_>>();
    let transaction_root = root
        .join("transactions")
        .join(format!("profile-import-{}", Uuid::new_v4()));
    fs::create_dir_all(&transaction_root).map_err(error_to_string)?;

    let bundle_file = File::open(bundle_path).map_err(error_to_string)?;
    let mut zip = ZipArchive::new(bundle_file).map_err(error_to_string)?;
    validate_zip_archive_safety(&mut zip)?;
    let manifest = read_bundle_manifest(&mut zip)?;
    let mut config_snapshots = Vec::new();
    for (index, config) in manifest.config_files.iter().enumerate() {
        let destination = safe_join(game_path, &config.target_relative_path)?;
        let snapshot = if destination.exists() {
            let snapshot = transaction_root.join(format!("config-{index}.bin"));
            fs::copy(&destination, &snapshot).map_err(error_to_string)?;
            Some(snapshot)
        } else {
            None
        };
        config_snapshots.push(DeploymentRollbackEntry {
            destination,
            immediate_backup: snapshot,
        });
    }
    drop(zip);

    let result = import_profile_bundle_impl_unchecked(root, bundle_path, game_path);
    if result.is_ok() {
        let _ = fs::remove_dir_all(transaction_root);
        return result;
    }

    let profiles_after = read_store::<GameProfile>(&profiles_file).unwrap_or(StoreFile {
        version: profiles_before.version,
        items: Vec::new(),
    });
    let mods_after = read_store::<InstalledModRecord>(&installed_mods_file).unwrap_or(StoreFile {
        version: mods_before.version,
        items: Vec::new(),
    });
    let added_profiles = profiles_after
        .items
        .iter()
        .filter(|profile| !existing_profile_ids.contains(&profile.id))
        .cloned()
        .collect::<Vec<_>>();
    for profile in &added_profiles {
        for record in mods_after.items.iter().filter(|record| {
            record.profile_id == profile.id && record.enabled && record.runtime_id.is_none()
        }) {
            let _ = deactivate_mod_files(root, profile, record);
        }
    }
    for snapshot in config_snapshots.iter().rev() {
        if let Some(source) = &snapshot.immediate_backup {
            let _ = replace_file_from_path(source, &snapshot.destination);
        } else if snapshot.destination.exists() {
            let _ = fs::remove_file(&snapshot.destination);
        }
    }
    let _ = write_store(&installed_mods_file, &mods_before);
    let _ = write_store(&profiles_file, &profiles_before);
    for profile in added_profiles {
        let _ = fs::remove_dir_all(profile_dir(root, &profile.id));
    }
    let _ = fs::remove_dir_all(transaction_root);
    result
}

#[allow(dead_code)]
fn import_profile_bundle_impl_unchecked(
    root: &Path,
    bundle_path: &Path,
    game_path: &Path,
) -> Result<ProfileImportResult, String> {
    if !bundle_path.is_file() {
        return Err(format!(
            "Profile bundle does not exist: {}",
            bundle_path.to_string_lossy()
        ));
    }
    if !game_path.is_dir() {
        return Err(format!(
            "Game folder does not exist: {}",
            game_path.to_string_lossy()
        ));
    }

    let bundle_file = File::open(bundle_path).map_err(error_to_string)?;
    let mut zip = ZipArchive::new(bundle_file).map_err(error_to_string)?;
    validate_zip_archive_safety(&mut zip)?;
    let manifest = read_bundle_manifest(&mut zip)?;
    if manifest.schema_version != 1 {
        return Err(format!(
            "Unsupported profile bundle version: {}",
            manifest.schema_version
        ));
    }
    let exported_profile = manifest.profile.clone();
    let bundle_mods = manifest.mods;
    let bundle_config_files = manifest.config_files;

    let steam_app_id = infer_steam_app_id_for_game_path(game_path);
    let detection = detect_game_setup_with_steam_app_id(game_path, steam_app_id.as_deref())?;
    let imported_at = now_string();
    let mut profile = exported_profile;
    profile.id = Uuid::new_v4().to_string();
    profile.name = unique_profile_name(root, &profile.name)?;
    profile.game_path = game_path.to_string_lossy().to_string();
    profile.game_id = detection.game_id.clone();
    profile.steam_app_id = steam_app_id;
    profile.engine = detection.engine.clone();
    profile.loader = detection.loader.clone();
    profile.created_at = imported_at.clone();
    profile.updated_at = imported_at;

    let profiles_file = profiles_path(root);
    let mut profiles = read_store::<GameProfile>(&profiles_file).map_err(error_to_string)?;
    profiles.items.push(profile.clone());
    write_store(&profiles_file, &profiles).map_err(error_to_string)?;
    fs::create_dir_all(profile_dir(root, &profile.id)).map_err(error_to_string)?;

    let mut warnings = install_profile_bootstrap_dependencies(root, &profile);
    let mut id_map = HashMap::new();
    let mut installed_mods = Vec::new();
    let mut deployed_files = Vec::new();

    for bundle_mod in bundle_mods {
        let old_mod_id = bundle_mod.record.id.clone();
        let new_mod_id = Uuid::new_v4().to_string();
        let package_source = extract_profile_package_source(
            &mut zip,
            &old_mod_id,
            &new_mod_id,
            &profile,
            root,
            &bundle_mod,
        )?;

        let mut record = bundle_mod.record;
        record.id = new_mod_id.clone();
        record.profile_id = profile.id.clone();
        record.archive_path = package_source.to_string_lossy().to_string();
        record.installed_at = now_string();
        record.files_written = Vec::new();
        record.backups_written = Vec::new();
        record.config_files = Vec::new();
        record.dependencies = record
            .dependencies
            .iter()
            .map(|dependency| refresh_dependency_status(root, &profile, dependency))
            .collect();

        if record.enabled {
            if let Some(plan) = record.plan.clone() {
                if let Some(reason) = incompatible_install_plan_reason(&profile, &plan) {
                    record.enabled = false;
                    record.last_status = "failed".to_string();
                    warnings.push(format!(
                        "Skipped {}: {}",
                        display_record_name(&record),
                        reason
                    ));
                    add_installed_mod(root, record.clone())?;
                    write_receipt(root, &profile, &record)?;
                    id_map.insert(old_mod_id, new_mod_id);
                    installed_mods.push(record);
                    continue;
                }

                let mut visited_dependencies = HashSet::new();
                match install_dependencies_for_plan(
                    root,
                    &profile,
                    &plan,
                    &mut visited_dependencies,
                    0,
                ) {
                    Ok(mut dependency_warnings) => warnings.append(&mut dependency_warnings),
                    Err(error) => warnings.push(format!(
                        "Could not install dependencies for {}: {}",
                        display_record_name(&record),
                        error
                    )),
                }

                match deploy_mod_files(
                    root,
                    &profile,
                    &new_mod_id,
                    &record.archive_path.clone(),
                    &plan,
                    &mut record,
                ) {
                    Ok(files) => {
                        record.files_written = files.clone();
                        record.config_files = config_files_from_paths(&record.files_written);
                        record.last_status = "installed".to_string();
                        deployed_files.extend(files);
                    }
                    Err(error) => {
                        record.enabled = false;
                        record.last_status = "failed".to_string();
                        warnings.push(format!(
                            "Could not deploy {}: {}",
                            display_record_name(&record),
                            error
                        ));
                    }
                }
            } else {
                record.enabled = false;
                record.last_status = "failed".to_string();
                warnings.push(format!(
                    "Could not deploy {} because no install plan was stored.",
                    display_record_name(&record)
                ));
            }
        } else {
            record.last_status = "disabled".to_string();
        }

        add_installed_mod(root, record.clone())?;
        write_receipt(root, &profile, &record)?;
        id_map.insert(old_mod_id, new_mod_id);
        installed_mods.push(record);
    }

    let mut config_files_written = Vec::new();
    for config_file in bundle_config_files {
        let Some(new_mod_id) = id_map.get(&config_file.mod_id) else {
            continue;
        };
        let destination_path = safe_join(game_path, &config_file.target_relative_path)?;
        if let Some(parent) = destination_path.parent() {
            fs::create_dir_all(parent).map_err(error_to_string)?;
        }
        extract_zip_file(&mut zip, &config_file.bundle_path, &destination_path)?;
        let written_config_path = destination_path.to_string_lossy().to_string();
        config_files_written.push(written_config_path.clone());

        for record in &mut installed_mods {
            if record.id == *new_mod_id && !record.config_files.contains(&written_config_path) {
                record.config_files.push(written_config_path.clone());
            }
        }
    }

    if !config_files_written.is_empty() {
        let installed_mods_file = installed_mods_path(root);
        let mut store =
            read_store::<InstalledModRecord>(&installed_mods_file).map_err(error_to_string)?;
        for record in &installed_mods {
            if let Some(stored_record) = store.items.iter_mut().find(|item| item.id == record.id) {
                stored_record.config_files = record.config_files.clone();
            }
        }
        write_store(&installed_mods_file, &store).map_err(error_to_string)?;
    }

    Ok(ProfileImportResult {
        profile,
        installed_mods,
        deployed_files,
        config_files_written,
        warnings,
    })
}

fn install_profile_bootstrap_dependencies(root: &Path, profile: &GameProfile) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut visited_dependencies = HashSet::new();
    let mut seen_dependencies = HashSet::new();

    for dependency in profile_bootstrap_dependencies(profile)
        .into_iter()
        .map(|dependency| refresh_dependency_status(root, profile, &dependency))
    {
        if !seen_dependencies.insert(dependency_key(&dependency))
            || dependency.status == "already-installed"
        {
            continue;
        }

        match install_dependency_by_provider(
            root,
            profile,
            &dependency,
            &mut visited_dependencies,
            0,
        ) {
            Ok(mut dependency_warnings) => warnings.append(&mut dependency_warnings),
            Err(error) => warnings.push(format!(
                "Could not install {} automatically: {}",
                dependency.name, error
            )),
        }
    }

    warnings
}

#[allow(dead_code)]
fn read_bundle_manifest(zip: &mut ZipArchive<File>) -> Result<ProfileBundleManifest, String> {
    let manifest_file = zip
        .by_name("manifest.json")
        .map_err(|_| "Profile bundle is missing manifest.json.".to_string())?;
    serde_json::from_reader(manifest_file).map_err(error_to_string)
}

#[allow(dead_code)]
fn extract_profile_package_source(
    zip: &mut ZipArchive<File>,
    old_mod_id: &str,
    new_mod_id: &str,
    profile: &GameProfile,
    root: &Path,
    bundle_mod: &ProfileBundleMod,
) -> Result<PathBuf, String> {
    let package_root = profile_package_dir(root, &profile.id, new_mod_id);
    fs::create_dir_all(&package_root).map_err(error_to_string)?;

    let source_relative_path = sanitize_bundle_relative_path(&bundle_mod.source_relative_path)?;
    let source_path = safe_join(&package_root, &source_relative_path)?;
    let bundle_entry = format!(
        "packages/{}/{}",
        sanitize_file_segment(old_mod_id),
        source_relative_path
    );

    if bundle_mod.source_is_directory {
        extract_zip_prefix(zip, &bundle_entry, &source_path)?;
    } else {
        extract_zip_file(zip, &bundle_entry, &source_path)?;
    }

    Ok(source_path)
}

fn add_directory_to_zip<W>(
    zip: &mut ZipWriter<W>,
    source_root: &Path,
    entry_root: &str,
) -> Result<(), String>
where
    W: Write + Seek,
{
    let entry_root = normalize_zip_entry(entry_root);
    zip.add_directory(format!("{entry_root}/"), zip_file_options())
        .map_err(error_to_string)?;

    let mut queue = VecDeque::from([source_root.to_path_buf()]);
    while let Some(current_path) = queue.pop_front() {
        for entry in fs::read_dir(&current_path).map_err(error_to_string)? {
            let entry = entry.map_err(error_to_string)?;
            let file_type = entry.file_type().map_err(error_to_string)?;
            if file_type.is_symlink() {
                continue;
            }

            let source_path = entry.path();
            let relative_path = source_path
                .strip_prefix(source_root)
                .map_err(error_to_string)?;
            let zip_entry = format!("{entry_root}/{}", to_portable_path(relative_path));

            if file_type.is_dir() {
                zip.add_directory(
                    format!("{}/", normalize_zip_entry(&zip_entry)),
                    zip_file_options(),
                )
                .map_err(error_to_string)?;
                queue.push_back(source_path);
            } else if file_type.is_file() {
                add_file_to_zip(zip, &source_path, &zip_entry)?;
            }
        }
    }

    Ok(())
}

fn add_file_to_zip<W>(
    zip: &mut ZipWriter<W>,
    source_path: &Path,
    entry_name: &str,
) -> Result<(), String>
where
    W: Write + Seek,
{
    zip.start_file(normalize_zip_entry(entry_name), zip_file_options())
        .map_err(error_to_string)?;
    let mut source_file = File::open(source_path).map_err(error_to_string)?;
    io::copy(&mut source_file, zip).map_err(error_to_string)?;
    Ok(())
}

fn add_bytes_to_zip<W>(zip: &mut ZipWriter<W>, entry_name: &str, bytes: &[u8]) -> Result<(), String>
where
    W: Write + Seek,
{
    zip.start_file(normalize_zip_entry(entry_name), zip_file_options())
        .map_err(error_to_string)?;
    zip.write_all(bytes).map_err(error_to_string)
}

#[allow(dead_code)]
fn extract_zip_prefix(
    zip: &mut ZipArchive<File>,
    entry_prefix: &str,
    destination_root: &Path,
) -> Result<(), String> {
    let prefix = format!(
        "{}/",
        normalize_zip_entry(entry_prefix).trim_end_matches('/')
    );
    let mut entry_names = Vec::new();

    for index in 0..zip.len() {
        let file = zip.by_index(index).map_err(error_to_string)?;
        let name = file.name().to_string();
        if name.starts_with(&prefix) {
            entry_names.push(name);
        }
    }

    if entry_names.is_empty() {
        return Err(format!(
            "Profile bundle is missing package source: {entry_prefix}"
        ));
    }

    for entry_name in entry_names {
        let relative_path = entry_name.trim_start_matches(&prefix);
        if relative_path.is_empty() {
            continue;
        }

        let destination_path = safe_join(destination_root, relative_path)?;
        if entry_name.ends_with('/') {
            fs::create_dir_all(destination_path).map_err(error_to_string)?;
        } else {
            extract_zip_file(zip, &entry_name, &destination_path)?;
        }
    }

    Ok(())
}

#[allow(dead_code)]
fn extract_zip_file(
    zip: &mut ZipArchive<File>,
    entry_name: &str,
    destination_path: &Path,
) -> Result<(), String> {
    let mut zip_file = zip
        .by_name(&normalize_zip_entry(entry_name))
        .map_err(error_to_string)?;
    if zip_file.name().ends_with('/') {
        return Err(format!("Expected a file but found a folder: {entry_name}"));
    }
    validate_archive_relative_path(entry_name)?;
    validate_archive_file_size(entry_name, zip_file.size(), zip_file.compressed_size())?;
    if let Some(parent) = destination_path.parent() {
        fs::create_dir_all(parent).map_err(error_to_string)?;
    }
    let temporary = destination_path.with_file_name(format!(
        ".{}.{}.extracting",
        destination_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("bundle-file"),
        Uuid::new_v4()
    ));
    let result = (|| -> Result<(), String> {
        let mut output = File::create(&temporary).map_err(error_to_string)?;
        copy_with_limit(&mut zip_file, &mut output, MAX_ARCHIVE_FILE_BYTES)?;
        output.sync_all().map_err(error_to_string)?;
        replace_file_from_path(&temporary, destination_path)
    })();
    if temporary.exists() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[allow(dead_code)]
fn validate_zip_archive_safety(zip: &mut ZipArchive<File>) -> Result<(), String> {
    if zip.len() > MAX_ARCHIVE_ENTRIES {
        return Err(format!(
            "Archive contains too many entries (maximum {}).",
            MAX_ARCHIVE_ENTRIES
        ));
    }
    let mut expanded_bytes = 0_u64;
    for index in 0..zip.len() {
        let file = zip.by_index(index).map_err(error_to_string)?;
        validate_archive_relative_path(file.name())?;
        if file.is_dir() {
            continue;
        }
        validate_archive_file_size(file.name(), file.size(), file.compressed_size())?;
        expanded_bytes = expanded_bytes
            .checked_add(file.size())
            .ok_or_else(|| "Archive expanded size overflowed its safety limit.".to_string())?;
        if expanded_bytes > MAX_ARCHIVE_EXPANDED_BYTES {
            return Err("Archive exceeds the expanded-size safety limit.".to_string());
        }
    }
    Ok(())
}

fn zip_file_options() -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644)
}

fn normalize_zip_entry(entry_name: &str) -> String {
    entry_name
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_string()
}

#[allow(dead_code)]
fn sanitize_bundle_relative_path(relative_path: &str) -> Result<String, String> {
    let normalized = normalize_archive_path(relative_path);
    let path = Path::new(&normalized);
    if normalized.is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!("Unsafe bundle path: {relative_path}"));
    }

    Ok(normalized)
}

#[allow(dead_code)]
fn unique_profile_name(root: &Path, base_name: &str) -> Result<String, String> {
    let profiles = read_store::<GameProfile>(&profiles_path(root))
        .map_err(error_to_string)?
        .items;
    let existing_names = profiles
        .iter()
        .map(|profile| profile.name.to_lowercase())
        .collect::<HashSet<_>>();

    if !existing_names.contains(&base_name.to_lowercase()) {
        return Ok(base_name.to_string());
    }

    let imported_name = format!("{base_name} Imported");
    if !existing_names.contains(&imported_name.to_lowercase()) {
        return Ok(imported_name);
    }

    for index in 2..1000 {
        let candidate = format!("{base_name} Imported {index}");
        if !existing_names.contains(&candidate.to_lowercase()) {
            return Ok(candidate);
        }
    }

    Err("Could not create a unique imported profile name.".to_string())
}

fn display_record_name(record: &InstalledModRecord) -> String {
    record
        .display_name
        .clone()
        .unwrap_or_else(|| record.archive_name.clone())
}

#[tauri::command]
async fn detect_game_setup(game_path: String) -> Result<GameDetectionResult, String> {
    tauri::async_runtime::spawn_blocking(move || detect_game_setup_impl(Path::new(&game_path)))
        .await
        .map_err(error_to_string)?
}

#[tauri::command]
async fn analyze_archive_for_profile(
    app: AppHandle,
    profile_id: String,
    archive_path: String,
) -> Result<ArchiveAnalysis, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = store_root(&app)?;
        let profile = get_profile(&root, &profile_id)?;
        ensure_verified_steam_profile(&profile)?;
        let scanned = scan_import_source(&root, Path::new(&archive_path))?;
        Ok(analyze_scanned_archive(scanned, &profile))
    })
    .await
    .map_err(error_to_string)?
}

#[tauri::command]
async fn install_archive(app: AppHandle, request: InstallRequest) -> Result<InstallResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = lock_mutations()?;
        let root = store_root(&app)?;
        let profile = get_profile(&root, &request.profile_id)?;
        ensure_verified_steam_profile(&profile)?;
        install_archive_impl(
            &root,
            &profile,
            &request.archive_path,
            request.archive_name.as_deref(),
            request.package_identity,
            &request.plan,
        )
    })
    .await
    .map_err(error_to_string)?
}

#[tauri::command]
async fn discover_online_mods(
    app: AppHandle,
    profile_id: String,
    page: usize,
    page_size: usize,
    sort: String,
    query: String,
) -> Result<DiscoveryPage, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = store_root(&app)?;
        let profile = get_profile(&root, &profile_id)?;
        ensure_verified_steam_profile(&profile)?;
        let settings = read_app_settings(&root)?;
        discover_online_mods_for_profile(&root, &profile, &settings, page, page_size, &sort, &query)
    })
    .await
    .map_err(error_to_string)?
}

#[tauri::command]
async fn install_discovered_mod(
    app: AppHandle,
    profile_id: String,
    provider: String,
    mod_id: String,
    version: Option<String>,
    provider_game_id: Option<String>,
    selected_file_id: Option<String>,
) -> Result<InstallResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = lock_mutations()?;
        let root = store_root(&app)?;
        let profile = get_profile(&root, &profile_id)?;
        ensure_verified_steam_profile(&profile)?;
        let settings = read_app_settings(&root)?;

        match provider.as_str() {
            "thunderstore" => install_thunderstore_discovered_mod(
                &root,
                &profile,
                &mod_id,
                version.clone(),
                provider_game_id.as_deref(),
            ),
            "nexus" => install_nexus_discovered_mod(
                &root,
                &profile,
                &mod_id,
                &settings,
                version,
                provider_game_id.as_deref(),
                selected_file_id.as_deref(),
            ),
            _ => Err(format!(
                "{} discovery install is not supported in this build.",
                provider
            )),
        }
    })
    .await
    .map_err(error_to_string)?
}

#[tauri::command]
async fn list_discovered_mod_files(
    app: AppHandle,
    profile_id: String,
    provider: String,
    mod_id: String,
    provider_game_id: Option<String>,
) -> Result<Vec<OnlineModFileOption>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = store_root(&app)?;
        let profile = get_profile(&root, &profile_id)?;
        ensure_verified_steam_profile(&profile)?;
        let settings = read_app_settings(&root)?;
        list_discovered_mod_files_impl(
            &profile,
            &settings,
            &provider,
            &mod_id,
            provider_game_id.as_deref(),
        )
    })
    .await
    .map_err(error_to_string)?
}

#[tauri::command]
async fn preflight_discovered_mod_install(
    app: AppHandle,
    profile_id: String,
    provider: String,
    mod_id: String,
) -> Result<InstallPreflightResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = store_root(&app)?;
        let profile = get_profile(&root, &profile_id)?;
        ensure_verified_steam_profile(&profile)?;
        if provider != "nexus" {
            return Ok(InstallPreflightResult {
                dependencies: Vec::new(),
                missing_dependencies: Vec::new(),
                confirmation_required: false,
            });
        }

        let (domain, nexus_mod_id) = parse_nexus_online_mod_id(&mod_id)?;
        verified_discovery_provider_game(&profile, "nexus", Some(&domain), Some(&domain))?;
        let settings = read_app_settings(&root)?;
        let client = provider_client()?;
        let requirements = fetch_nexus_mod_requirements(
            &client,
            &profile,
            &domain,
            nexus_mod_id,
            settings.nexus_api_key(),
        )?;
        let dependencies = nexus_requirement_dependencies(&profile, &domain, &requirements)
            .into_iter()
            .map(|dependency| refresh_dependency_status(&root, &profile, &dependency))
            .collect::<Vec<_>>();
        let missing_dependencies = dependencies
            .iter()
            .filter(|dependency| dependency.required && dependency.status != "already-installed")
            .cloned()
            .collect::<Vec<_>>();
        let confirmation_required = missing_dependencies
            .iter()
            .any(|dependency| matches!(dependency.provider.as_str(), "nexus" | "manual"));

        Ok(InstallPreflightResult {
            dependencies,
            missing_dependencies,
            confirmation_required,
        })
    })
    .await
    .map_err(error_to_string)?
}

#[tauri::command]
async fn begin_nexus_requirement_download(
    app: AppHandle,
    profile_id: String,
    dependency_id: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = lock_mutations()?;
        let root = store_root(&app)?;
        let profile = get_profile(&root, &profile_id)?;
        ensure_verified_steam_profile(&profile)?;
        let dependency = DependencySpec {
            id: dependency_id.clone(),
            name: dependency_id.clone(),
            version: None,
            provider: "nexus".to_string(),
            required: true,
            status: "missing".to_string(),
            source: None,
            notes: None,
        };
        if dependency_already_available(&root, &profile, &dependency) {
            return Err("That Nexus requirement is already installed in this profile.".to_string());
        }
        let settings = read_app_settings(&root)?;
        let api_key = settings.nexus_api_key().ok_or_else(|| {
            "Add your Nexus API key in Settings before downloading requirements.".to_string()
        })?;
        let (domain, nexus_mod_id) = parse_nexus_online_mod_id(&dependency_id)?;
        let provider_game_id = verified_discovery_provider_game(
            &profile,
            "nexus",
            Some(&domain),
            Some(&domain),
        )?;
        let client = provider_client()?;
        let nested_requirements = fetch_nexus_mod_requirements(
            &client,
            &profile,
            &domain,
            nexus_mod_id,
            Some(api_key),
        )?;
        let missing_confirmation_dependency =
            nexus_requirement_dependencies(&profile, &domain, &nested_requirements)
        .into_iter()
        .map(|dependency| refresh_dependency_status(&root, &profile, &dependency))
        .find(|dependency| {
            dependency.required
                && dependency.status != "already-installed"
                && matches!(dependency.provider.as_str(), "nexus" | "manual")
        });
        if let Some(dependency) = missing_confirmation_dependency {
            return Err(format!(
                "{} requires {} first. Confirm the missing requirement in UniLoader before starting the parent download.",
                profile_game_label(&profile), dependency.name
            ));
        }
        let files = fetch_nexus_mod_files(&client, api_key, &domain, nexus_mod_id)?;
        let file = choose_nexus_file(&files).ok_or_else(|| {
            "Nexus returned no supported main archive for this requirement.".to_string()
        })?;
        let now = Utc::now().timestamp();
        store_pending_nexus_download(
            &root,
            PendingNexusDownload {
                profile_id,
                domain: domain.clone(),
                mod_id: nexus_mod_id,
                file_id: file.file_id,
                version: file.version.clone(),
                provider_game_id,
                created_at: now,
            },
            now,
        )?;
        Ok(nexus_manager_download_page_url(
            &domain,
            nexus_mod_id,
            file.file_id,
        ))
    })
    .await
    .map_err(error_to_string)?
}

#[tauri::command]
async fn begin_nexus_browser_download(
    app: AppHandle,
    profile_id: String,
    mod_id: String,
    version: Option<String>,
    provider_game_id: Option<String>,
    selected_file_id: String,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = lock_mutations()?;
        let root = store_root(&app)?;
        let profile = get_profile(&root, &profile_id)?;
        ensure_verified_steam_profile(&profile)?;
        let settings = read_app_settings(&root)?;
        let api_key = settings.nexus_api_key().ok_or_else(|| {
            "Add your Nexus API key in Settings before downloading Nexus mods.".to_string()
        })?;
        let (domain, nexus_mod_id) = parse_nexus_online_mod_id(&mod_id)?;
        let verified_provider_game_id = verified_discovery_provider_game(
            &profile,
            "nexus",
            provider_game_id.as_deref(),
            Some(&domain),
        )?;
        let client = provider_client()?;
        let files = fetch_nexus_mod_files(&client, api_key, &domain, nexus_mod_id)?;
        let file = choose_requested_nexus_file(&files, Some(&selected_file_id))?
            .ok_or_else(|| "The selected Nexus file is no longer available.".to_string())?;
        let now = Utc::now().timestamp();
        let pending = PendingNexusDownload {
            profile_id,
            domain: domain.clone(),
            mod_id: nexus_mod_id,
            file_id: file.file_id,
            version: version.or_else(|| file.version.clone()),
            provider_game_id: verified_provider_game_id,
            created_at: now,
        };
        store_pending_nexus_download(&root, pending, now)?;

        Ok(nexus_manager_download_page_url(
            &domain,
            nexus_mod_id,
            file.file_id,
        ))
    })
    .await
    .map_err(error_to_string)?
}

#[tauri::command]
async fn install_nexus_nxm_link(
    app: AppHandle,
    nxm_url: String,
) -> Result<NexusNxmInstallResult, String> {
    reveal_main_window(&app);
    let install_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let _operation = lock_mutations()?;
        let root = store_root(&install_app)?;
        let settings = read_app_settings(&root)?;
        let api_key = settings.nexus_api_key().ok_or_else(|| {
            "Your Nexus API key is missing. Add it in Settings and try the download again."
                .to_string()
        })?;
        let nxm = parse_nexus_nxm_link(&nxm_url)?;
        let pending = find_pending_nexus_download(&root, &nxm)?;
        let profile = get_profile(&root, &pending.profile_id)?;
        ensure_verified_steam_profile(&profile)?;
        let provider_game_id = verified_discovery_provider_game(
            &profile,
            "nexus",
            Some(&pending.provider_game_id),
            Some(&nxm.domain),
        )?;
        let client = provider_client()?;
        let account = validate_nexus_api_key(api_key)?;
        let files = fetch_nexus_mod_files(&client, api_key, &nxm.domain, nxm.mod_id)?;
        let file = choose_requested_nexus_file(&files, Some(&nxm.file_id.to_string()))?
            .ok_or_else(|| "The Nexus file from this download link is no longer available.".to_string())?;
        let links = fetch_nexus_download_links_with_nxm(&client, api_key, &nxm)?;
        let download_url = nexus_http_download_url(&links).ok_or_else(|| {
            "Nexus authorized the request but did not return a usable download server. Try the download again."
                .to_string()
        })?;
        let mut visited_dependencies = HashSet::new();
        let install_result = install_resolved_nexus_file(
            &root,
            &profile,
            &client,
            api_key,
            &account,
            &nxm.domain,
            nxm.mod_id,
            &file,
            &download_url,
            pending.version,
            provider_game_id,
            &mut visited_dependencies,
            0,
        )?;
        remove_pending_nexus_download(&root, &nxm)?;

        Ok(NexusNxmInstallResult {
            mod_id: format!("nexus:{}/{}", nxm.domain, nxm.mod_id),
            install_result,
        })
    })
    .await
    .map_err(error_to_string)?;
    reveal_main_window(&app);
    result
}

#[tauri::command]
fn list_installed_mods(
    app: AppHandle,
    profile_id: String,
) -> Result<Vec<InstalledModRecord>, String> {
    let _operation = lock_mutations()?;
    let root = store_root(&app)?;
    let profile = get_profile(&root, &profile_id)?;
    let store_path = installed_mods_path(&root);
    let mut store = read_store::<InstalledModRecord>(&store_path).map_err(error_to_string)?;
    let mut changed = ensure_visible_runtime_records(&root, &profile, &mut store)? > 0;
    let discovered_config_files = discover_profile_config_files(&profile);
    for record in store
        .items
        .iter_mut()
        .filter(|record| record.profile_id == profile_id)
    {
        let resolved = resolved_config_files_for_record(&profile, record, &discovered_config_files);
        if record.config_files != resolved {
            record.config_files = resolved;
            changed = true;
        }
    }
    if changed {
        write_store(&store_path, &store).map_err(error_to_string)?;
    }
    Ok(store
        .items
        .into_iter()
        .filter(|record| record.profile_id == profile_id)
        .map(|mut record| {
            record.display_name = record
                .display_name
                .as_deref()
                .map(humanize_mod_display_name);
            record
        })
        .collect())
}

#[tauri::command]
async fn get_mod_config_details(
    app: AppHandle,
    profile_id: String,
    installed_mod_id: String,
) -> Result<Vec<ModConfigFile>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        get_mod_config_details_sync(app, profile_id, installed_mod_id)
    })
    .await
    .map_err(error_to_string)?
}

fn get_mod_config_details_sync(
    app: AppHandle,
    profile_id: String,
    installed_mod_id: String,
) -> Result<Vec<ModConfigFile>, String> {
    let root = store_root(&app)?;
    let profile = get_profile(&root, &profile_id)?;
    let record = read_store::<InstalledModRecord>(&installed_mods_path(&root))
        .map_err(error_to_string)?
        .items
        .into_iter()
        .find(|record| record.id == installed_mod_id && record.profile_id == profile_id)
        .ok_or_else(|| format!("Installed mod not found: {}", installed_mod_id))?;
    let discovered_config_files = discover_profile_config_files(&profile);
    let config_files =
        resolved_config_files_for_record(&profile, &record, &discovered_config_files);

    config_files
        .iter()
        .map(|file_path| read_mod_config_file(&root, &profile, file_path))
        .collect()
}

#[tauri::command]
fn update_mod_config_value(
    app: AppHandle,
    input: UpdateModConfigValueInput,
) -> Result<ModConfigFile, String> {
    let _operation = lock_mutations()?;
    let root = store_root(&app)?;
    let profile = get_profile(&root, &input.profile_id)?;
    let path = PathBuf::from(&input.file_path);

    validate_config_file_for_edit(&root, &profile, &path)?;
    let content = fs::read_to_string(&path).map_err(error_to_string)?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_lowercase());
    let next_content = match extension.as_deref() {
        Some("json") => update_json_config_content(
            &content,
            input.section.as_deref(),
            &input.key,
            &input.value,
        )?,
        Some("toml") => update_toml_config_content(
            &content,
            input.section.as_deref(),
            &input.key,
            &input.value,
        )?,
        Some("yaml") | Some("yml") => update_yaml_config_content(
            &content,
            input.section.as_deref(),
            &input.key,
            &input.value,
        )?,
        _ => update_key_value_config_content(
            &content,
            input.section.as_deref(),
            &input.key,
            &input.value,
        )?,
    };

    validate_structured_config_content(extension.as_deref(), &next_content)?;
    atomic_write(&path, next_content.as_bytes()).map_err(error_to_string)?;
    read_mod_config_file(&root, &profile, &input.file_path)
}

#[tauri::command]
async fn disable_mod(
    app: AppHandle,
    profile_id: String,
    installed_mod_id: String,
) -> Result<ModActionResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        disable_mod_sync(app, profile_id, installed_mod_id)
    })
    .await
    .map_err(error_to_string)?
}

fn disable_mod_sync(
    app: AppHandle,
    profile_id: String,
    installed_mod_id: String,
) -> Result<ModActionResult, String> {
    let _operation = lock_mutations()?;
    let root = store_root(&app)?;
    let profile = get_profile(&root, &profile_id)?;
    let path = installed_mods_path(&root);
    let mut store = read_store::<InstalledModRecord>(&path).map_err(error_to_string)?;
    let record_index = store
        .items
        .iter()
        .position(|record| record.id == installed_mod_id && record.profile_id == profile_id)
        .ok_or_else(|| format!("Installed mod not found: {installed_mod_id}"))?;
    let original = store.items[record_index].clone();
    if original.runtime_id.is_some() {
        return Err(
            "System runtimes stay enabled because installed mods may depend on them.".to_string(),
        );
    }
    if !original.enabled {
        return Ok(ModActionResult {
            profile_id,
            installed_mod_id,
            status: original.last_status,
            files_changed: Vec::new(),
            warnings: vec!["Mod is already disabled.".to_string()],
        });
    }

    let files_changed = deactivate_mod_files(&root, &profile, &original)?;
    let mut updated = original.clone();
    updated.enabled = false;
    updated.last_status = "disabled".to_string();
    store.items[record_index] = updated.clone();

    if let Err(error) = write_receipt(&root, &profile, &updated) {
        restore_deactivated_mods(&root, &profile, std::slice::from_ref(&original));
        return Err(error);
    }
    if let Err(error) = write_store(&path, &store) {
        restore_deactivated_mods(&root, &profile, std::slice::from_ref(&original));
        let _ = write_receipt(&root, &profile, &original);
        return Err(error_to_string(error));
    }

    Ok(ModActionResult {
        profile_id,
        installed_mod_id,
        status: "disabled".to_string(),
        files_changed,
        warnings: Vec::new(),
    })
}

#[tauri::command]
async fn enable_mod(
    app: AppHandle,
    profile_id: String,
    installed_mod_id: String,
) -> Result<ModActionResult, String> {
    tauri::async_runtime::spawn_blocking(move || enable_mod_sync(app, profile_id, installed_mod_id))
        .await
        .map_err(error_to_string)?
}

fn enable_mod_sync(
    app: AppHandle,
    profile_id: String,
    installed_mod_id: String,
) -> Result<ModActionResult, String> {
    let _operation = lock_mutations()?;
    let root = store_root(&app)?;
    let profile = get_profile(&root, &profile_id)?;
    let path = installed_mods_path(&root);
    let mut store = read_store::<InstalledModRecord>(&path).map_err(error_to_string)?;
    let record_index = store
        .items
        .iter()
        .position(|record| record.id == installed_mod_id && record.profile_id == profile_id)
        .ok_or_else(|| format!("Installed mod not found: {installed_mod_id}"))?;
    let original = store.items[record_index].clone();
    if original.runtime_id.is_some() {
        return Err("System runtimes are already managed automatically.".to_string());
    }
    if original.enabled {
        return Ok(ModActionResult {
            profile_id,
            installed_mod_id,
            status: original.last_status,
            files_changed: Vec::new(),
            warnings: vec!["Mod is already enabled.".to_string()],
        });
    }

    let plan = original.plan.clone().ok_or_else(|| {
        "This older install record cannot be re-enabled because it has no install plan.".to_string()
    })?;
    let deployment = deploy_plan_transaction(
        &root,
        &profile,
        &original.id,
        Path::new(&original.archive_path),
        &plan,
        Some(&original.id),
    )?;
    let mut updated = original.clone();
    updated.enabled = true;
    updated.last_status = "installed".to_string();
    updated.files_written = deployment.files_written.clone();
    updated.backups_written = deployment.backups_written.clone();
    updated.written_file_hashes = deployment.written_file_hashes.clone();
    updated.config_files = config_files_from_paths(&updated.files_written);
    store.items[record_index] = updated.clone();

    if let Err(error) = write_receipt(&root, &profile, &updated) {
        deployment.rollback();
        return Err(error);
    }
    if let Err(error) = write_store(&path, &store) {
        deployment.rollback();
        let _ = write_receipt(&root, &profile, &original);
        return Err(error_to_string(error));
    }
    let files_changed = deployment.files_written.clone();
    deployment.commit();

    Ok(ModActionResult {
        profile_id,
        installed_mod_id,
        status: "installed".to_string(),
        files_changed,
        warnings: Vec::new(),
    })
}

#[tauri::command]
async fn set_all_profile_mods_enabled(
    app: AppHandle,
    profile_id: String,
    enabled: bool,
) -> Result<ProfileModToggleResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        set_all_profile_mods_enabled_sync(app, profile_id, enabled)
    })
    .await
    .map_err(error_to_string)?
}

fn set_all_profile_mods_enabled_sync(
    app: AppHandle,
    profile_id: String,
    enabled: bool,
) -> Result<ProfileModToggleResult, String> {
    let _operation = lock_mutations()?;
    let root = store_root(&app)?;
    let profile = get_profile(&root, &profile_id)?;
    let store_path = installed_mods_path(&root);
    let original_store = read_store::<InstalledModRecord>(&store_path).map_err(error_to_string)?;
    let mut next_store = StoreFile {
        version: original_store.version,
        items: original_store.items.clone(),
    };
    let target_indices = next_store
        .items
        .iter()
        .enumerate()
        .filter(|(_, record)| {
            record.profile_id == profile_id
                && record.runtime_id.is_none()
                && record.enabled != enabled
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let mut files_changed = Vec::new();
    let mut deployments = Vec::<DeploymentOutcome>::new();
    let mut deactivated = Vec::<InstalledModRecord>::new();

    if enabled {
        for index in &target_indices {
            next_store.items[*index].enabled = true;
        }
        let pending_records = target_indices
            .iter()
            .map(|index| next_store.items[*index].clone())
            .collect::<Vec<_>>();
        validate_profile_migration_plans(&profile, &pending_records)?;

        for index in &target_indices {
            let original = &original_store.items[*index];
            let plan = original.plan.as_ref().ok_or_else(|| {
                format!(
                    "{} cannot be enabled because its install plan is unavailable.",
                    display_record_name(original)
                )
            })?;
            let deployment = match deploy_plan_transaction(
                &root,
                &profile,
                &original.id,
                Path::new(&original.archive_path),
                plan,
                Some(&original.id),
            ) {
                Ok(deployment) => deployment,
                Err(error) => {
                    for deployment in deployments.iter().rev() {
                        deployment.rollback();
                    }
                    return Err(format!("{}: {error}", display_record_name(original)));
                }
            };
            let updated = &mut next_store.items[*index];
            updated.enabled = true;
            updated.last_status = "installed".to_string();
            updated.files_written = deployment.files_written.clone();
            updated.backups_written = deployment.backups_written.clone();
            updated.written_file_hashes = deployment.written_file_hashes.clone();
            updated.config_files = config_files_from_paths(&updated.files_written);
            files_changed.extend(deployment.files_written.clone());
            deployments.push(deployment);
        }
    } else {
        for index in &target_indices {
            let original = original_store.items[*index].clone();
            match deactivate_mod_files(&root, &profile, &original) {
                Ok(files) => files_changed.extend(files),
                Err(error) => {
                    restore_deactivated_mods(&root, &profile, &deactivated);
                    return Err(format!("{}: {error}", display_record_name(&original)));
                }
            }
            deactivated.push(original);
            next_store.items[*index].enabled = false;
            next_store.items[*index].last_status = "disabled".to_string();
        }
    }

    for index in &target_indices {
        if let Err(error) = write_receipt(&root, &profile, &next_store.items[*index]) {
            for deployment in deployments.iter().rev() {
                deployment.rollback();
            }
            restore_deactivated_mods(&root, &profile, &deactivated);
            for original in original_store
                .items
                .iter()
                .filter(|record| record.profile_id == profile_id)
            {
                let _ = write_receipt(&root, &profile, original);
            }
            return Err(error);
        }
    }
    if let Err(error) = write_store(&store_path, &next_store) {
        for deployment in deployments.iter().rev() {
            deployment.rollback();
        }
        restore_deactivated_mods(&root, &profile, &deactivated);
        for original in original_store
            .items
            .iter()
            .filter(|record| record.profile_id == profile_id)
        {
            let _ = write_receipt(&root, &profile, original);
        }
        return Err(error_to_string(error));
    }
    for deployment in deployments {
        deployment.commit();
    }

    let installed_mods = next_store
        .items
        .into_iter()
        .filter(|record| record.profile_id == profile_id)
        .collect();
    Ok(ProfileModToggleResult {
        profile_id,
        enabled,
        changed_mods: target_indices.len(),
        files_changed,
        warnings: Vec::new(),
        installed_mods,
    })
}

#[tauri::command]
async fn remove_mod(
    app: AppHandle,
    profile_id: String,
    installed_mod_id: String,
) -> Result<ModActionResult, String> {
    tauri::async_runtime::spawn_blocking(move || remove_mod_sync(app, profile_id, installed_mod_id))
        .await
        .map_err(error_to_string)?
}

fn remove_mod_sync(
    app: AppHandle,
    profile_id: String,
    installed_mod_id: String,
) -> Result<ModActionResult, String> {
    let _operation = lock_mutations()?;
    let root = store_root(&app)?;
    let profile = get_profile(&root, &profile_id)?;
    let path = installed_mods_path(&root);
    let mut store = read_store::<InstalledModRecord>(&path).map_err(error_to_string)?;
    let record_index = store
        .items
        .iter()
        .position(|record| record.id == installed_mod_id && record.profile_id == profile_id)
        .ok_or_else(|| format!("Installed mod not found: {}", installed_mod_id))?;
    if store.items[record_index].runtime_id.is_some() {
        return Err(
            "System runtimes are protected. UniLoader keeps them available for dependent mods."
                .to_string(),
        );
    }
    let record = store.items.remove(record_index);
    let files_changed = if record.enabled {
        deactivate_mod_files(&root, &profile, &record)?
    } else {
        Vec::new()
    };

    if let Err(error) = write_store(&path, &store) {
        if record.enabled {
            restore_deactivated_mods(&root, &profile, std::slice::from_ref(&record));
        }
        return Err(error_to_string(error));
    }
    cleanup_install_data(&root, &profile_id, &record.id);

    Ok(ModActionResult {
        profile_id,
        installed_mod_id,
        status: "removed".to_string(),
        files_changed,
        warnings: Vec::new(),
    })
}

#[tauri::command]
fn get_store_path(app: AppHandle) -> Result<String, String> {
    Ok(store_root(&app)?.to_string_lossy().to_string())
}

#[tauri::command]
fn open_profile_game_folder(app: AppHandle, profile_id: String) -> Result<(), String> {
    let root = store_root(&app)?;
    let profile = get_profile(&root, &profile_id)?;
    let game_path = Path::new(&profile.game_path);

    if !game_path.is_dir() {
        return Err(format!(
            "Game folder no longer exists: {}",
            profile.game_path
        ));
    }

    open_folder_in_shell(game_path)
}

#[tauri::command]
fn open_external_url(url: String) -> Result<(), String> {
    open_url_in_shell(&url)
}

#[tauri::command]
fn download_update_installer(
    app: AppHandle,
    url: String,
    file_name: Option<String>,
) -> Result<String, String> {
    let trimmed_url = validated_update_url(&url)?;
    let safe_name = update_installer_file_name(&trimmed_url, file_name.as_deref())?;
    let download_dir = update_download_dir()?;
    fs::create_dir_all(&download_dir).map_err(error_to_string)?;
    let destination = download_dir.join(safe_name);
    if destination.exists() {
        fs::remove_file(&destination).map_err(error_to_string)?;
    }
    let client = Client::builder()
        .timeout(Duration::from_secs(240))
        .build()
        .map_err(error_to_string)?;

    download_url_to_file(&client, &trimmed_url, &destination)?;
    let checksum_url = format!("{trimmed_url}.sha256");
    let expected_hash = download_update_checksum(&client, &checksum_url)?;
    let actual_hash = sha256_file(&destination)?;
    if !actual_hash.eq_ignore_ascii_case(&expected_hash) {
        let _ = fs::remove_file(&destination);
        return Err(
            "The downloaded installer failed its SHA-256 integrity check and was deleted."
                .to_string(),
        );
    }
    launch_update_installer(&destination)?;

    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(900));
        app.exit(0);
    });

    Ok(destination.to_string_lossy().to_string())
}

pub fn run() {
    let mut builder = tauri::Builder::default();

    #[cfg(windows)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            reveal_main_window(app);
        }));
    }

    builder
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .on_page_load(|webview, payload| {
            if !matches!(payload.event(), PageLoadEvent::Finished) {
                return;
            }

            let window = webview.window();
            if window.label() != "main" {
                return;
            }

            let _ = window.set_icon(APP_ICON.clone());
            let _ = window.show();
            let _ = window.unminimize();
        })
        .setup(|app| {
            #[cfg(windows)]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                app.deep_link().register_all()?;
            }
            if let Some(window) = app.get_webview_window("main") {
                window.set_icon(APP_ICON.clone())?;
            }
            setup_tray(app, APP_ICON.clone())?;
            refresh_windows_shell_icons_once(app.handle());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                let should_minimize = store_root(app)
                    .and_then(|root| read_app_settings(&root))
                    .map(|settings| settings.minimize_to_tray_on_close)
                    .unwrap_or(false);

                if should_minimize {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_app_settings,
            update_app_settings,
            save_nexus_api_key,
            scan_steam_games,
            create_steam_profile,
            launch_profile_game,
            profile_game_running,
            set_all_profile_mods_enabled,
            list_profiles,
            profile_folder_exists,
            create_profile,
            rename_profile,
            remove_profile,
            refresh_profile,
            bootstrap_profile_dependencies,
            update_profile_game_folder,
            export_profile_bundle,
            import_profile_bundle,
            detect_game_setup,
            analyze_archive_for_profile,
            install_archive,
            discover_online_mods,
            list_discovered_mod_files,
            preflight_discovered_mod_install,
            install_discovered_mod,
            begin_nexus_browser_download,
            begin_nexus_requirement_download,
            install_nexus_nxm_link,
            list_installed_mods,
            get_mod_config_details,
            update_mod_config_value,
            disable_mod,
            enable_mod,
            remove_mod,
            open_profile_game_folder,
            open_external_url,
            download_update_installer,
            check_app_update,
            get_store_path
        ])
        .run(tauri::generate_context!())
        .expect("failed to run UniLoader");
}

fn reveal_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg(windows)]
fn refresh_windows_shell_icons_once(app: &AppHandle) {
    use windows_sys::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};

    let Ok(app_data_dir) = app.path().app_data_dir() else {
        return;
    };
    let marker_path = app_data_dir.join("shell-icon-version.txt");
    let current_version = env!("CARGO_PKG_VERSION");
    if fs::read_to_string(&marker_path)
        .map(|value| value.trim() == current_version)
        .unwrap_or(false)
    {
        return;
    }

    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED as i32,
            SHCNF_IDLIST,
            std::ptr::null(),
            std::ptr::null(),
        );
    }

    if fs::create_dir_all(&app_data_dir).is_ok() {
        let _ = fs::write(marker_path, format!("{current_version}\n"));
    }
}

#[cfg(not(windows))]
fn refresh_windows_shell_icons_once(_app: &AppHandle) {}

fn setup_tray(app: &mut tauri::App, icon: tauri::image::Image<'static>) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, "tray-show", "Show UniLoader", true, None::<&str>)?;
    let hide_item = MenuItem::with_id(app, "tray-hide", "Hide to tray", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "tray-quit", "Quit UniLoader", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show_item, &hide_item, &separator, &quit_item])?;

    let tray = TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("UniLoader")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "tray-show" => reveal_main_window(app),
            "tray-hide" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            "tray-quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            let TrayIconEvent::Click {
                button,
                button_state,
                ..
            } = event
            else {
                return;
            };

            if button == MouseButton::Left && button_state == MouseButtonState::Up {
                reveal_main_window(tray.app_handle());
            }
        });

    let tray_icon = tray.build(app)?;
    app.manage(tray_icon);
    Ok(())
}

fn detect_game_setup_impl(game_path: &Path) -> Result<GameDetectionResult, String> {
    let steam_app_id = infer_steam_app_id_for_game_path(game_path);
    detect_game_setup_with_steam_app_id(game_path, steam_app_id.as_deref())
}

fn detect_game_setup_with_steam_app_id(
    game_path: &Path,
    steam_app_id: Option<&str>,
) -> Result<GameDetectionResult, String> {
    if !game_path.is_dir() {
        return Err("Selected game path must be a folder.".to_string());
    }

    let entries = walk_game_folder(game_path);
    let game_id = detect_game_id(&entries, steam_app_id);
    let mut signals = Vec::new();
    let mut engine_scores = score_map(&[
        "unity-mono",
        "unity-il2cpp",
        "unreal",
        "re-engine",
        "unknown",
    ]);
    let mut loader_scores = score_map(&[
        "none",
        "bepinex",
        "bepinex-il2cpp",
        "ue4ss",
        "reframework",
        "loose-files",
    ]);

    score_engine(&entries, &mut engine_scores, &mut signals);
    if let Some(definition) = game_id
        .as_deref()
        .and_then(|game_id| game_definition_by_id(game_id))
    {
        if let Some(engine) = definition.engine.as_deref() {
            add_score(
                &mut engine_scores,
                &mut signals,
                engine,
                38,
                "Known game engine",
                &definition.id,
            );
        }
    }
    let engine = choose_highest(&engine_scores, "unknown");
    score_loaders(&entries, &engine, &mut loader_scores, &mut signals);

    let installed_loader = choose_highest(&loader_scores, "none");
    let recommended_loader = recommend_loader(game_id.as_deref(), &engine, &entries);
    let loader_installed = installed_loader != "none"
        && loader_scores
            .get(&installed_loader)
            .copied()
            .unwrap_or_default()
            >= 25;
    let loader = if loader_installed {
        installed_loader
    } else {
        recommended_loader.clone()
    };

    let mut warnings = Vec::new();
    if engine == "unknown" {
        warnings.push("Engine could not be identified from this folder.".to_string());
    }

    if !loader_installed && recommended_loader != "none" {
        warnings.push(format!(
            "{} is recommended but not installed yet.",
            format_loader(&recommended_loader)
        ));
    }

    if entries.len() >= MAX_SCAN_ENTRIES {
        warnings
            .push("Detection stopped early because the folder contains many files.".to_string());
    }

    if let Some(definition) = game_id
        .as_deref()
        .and_then(|game_id| game_definition_by_id(game_id))
    {
        signals.push(DetectionSignal {
            label: format!("Known game profile: {}", definition.display_name),
            path: definition.id.clone(),
            weight: 12,
        });
        if steam_app_id.is_some_and(|app_id| {
            definition
                .steam_app_ids
                .iter()
                .any(|known_id| known_id == app_id.trim())
        }) {
            signals.push(DetectionSignal {
                label: "Steam App ID match".to_string(),
                path: steam_app_id.unwrap_or_default().trim().to_string(),
                weight: 50,
            });
        }
    }

    let route_preparation =
        prepare_mod_routes(game_path, game_id.as_deref(), &engine, &loader, &entries);
    warnings.extend(route_preparation.warnings.clone());

    Ok(GameDetectionResult {
        game_path: game_path.to_string_lossy().to_string(),
        game_id,
        engine: engine.clone(),
        loader,
        recommended_loader,
        engine_confidence: confidence_for(engine_scores.get(&engine).copied().unwrap_or_default()),
        loader_confidence: if loader_installed {
            confidence_for(
                loader_scores
                    .get(&choose_highest(&loader_scores, "none"))
                    .copied()
                    .unwrap_or_default(),
            )
        } else {
            0.0
        },
        loader_installed,
        expected_mod_folders: route_preparation.expected_mod_folders,
        created_mod_folders: route_preparation.created_mod_folders,
        signals,
        warnings,
    })
}

fn score_engine(
    entries: &[ProbeEntry],
    scores: &mut HashMap<String, i32>,
    signals: &mut Vec<DetectionSignal>,
) {
    for entry in entries {
        let lower_path = entry.relative_path.to_lowercase();
        let lower_name = entry.name.to_lowercase();

        if entry.is_directory && lower_name.ends_with("_data") && entry.depth <= 4 {
            add_score(
                scores,
                signals,
                "unity-mono",
                28,
                "Unity data folder",
                &entry.relative_path,
            );
            add_score(
                scores,
                signals,
                "unity-il2cpp",
                28,
                "Unity data folder",
                &entry.relative_path,
            );
        }

        if !entry.is_directory && lower_name == "unityplayer.dll" && entry.depth <= 4 {
            add_score(
                scores,
                signals,
                "unity-mono",
                24,
                "Unity player runtime",
                &entry.relative_path,
            );
            add_score(
                scores,
                signals,
                "unity-il2cpp",
                24,
                "Unity player runtime",
                &entry.relative_path,
            );
        }

        if !entry.is_directory && lower_name == "gameassembly.dll" {
            add_score(
                scores,
                signals,
                "unity-il2cpp",
                45,
                "Unity IL2CPP game assembly",
                &entry.relative_path,
            );
        }

        if lower_path.contains("/il2cpp_data/") || lower_path.ends_with("/il2cpp_data") {
            add_score(
                scores,
                signals,
                "unity-il2cpp",
                22,
                "Unity IL2CPP data folder",
                &entry.relative_path,
            );
        }

        if !entry.is_directory && lower_path.ends_with("/managed/assembly-csharp.dll") {
            add_score(
                scores,
                signals,
                "unity-mono",
                45,
                "Unity managed game assembly",
                &entry.relative_path,
            );
        }

        if entry.is_directory && lower_name == "monobleedingedge" {
            add_score(
                scores,
                signals,
                "unity-mono",
                18,
                "Unity Mono runtime folder",
                &entry.relative_path,
            );
        }

        if lower_path == "binaries/win64"
            || lower_path.ends_with("/binaries/win64")
            || lower_path.contains("/binaries/win64/")
        {
            add_score(
                scores,
                signals,
                "unreal",
                32,
                "Unreal Win64 binaries folder",
                &entry.relative_path,
            );
        }

        if lower_path == "content/paks"
            || lower_path.ends_with("/content/paks")
            || lower_path.contains("/content/paks/")
        {
            add_score(
                scores,
                signals,
                "unreal",
                38,
                "Unreal pak folder",
                &entry.relative_path,
            );
        }

        if !entry.is_directory && lower_name.ends_with(".uproject") {
            add_score(
                scores,
                signals,
                "unreal",
                26,
                "Unreal project file",
                &entry.relative_path,
            );
        }

        if !entry.is_directory
            && lower_name.ends_with(".pak")
            && lower_path.contains("/content/paks/")
        {
            add_score(
                scores,
                signals,
                "unreal",
                24,
                "Unreal pak file",
                &entry.relative_path,
            );
        }

        if !entry.is_directory
            && lower_name.starts_with("re_chunk_")
            && lower_name.ends_with(".pak")
        {
            add_score(
                scores,
                signals,
                "re-engine",
                48,
                "RE Engine chunk pak",
                &entry.relative_path,
            );
        }

        if entry.is_directory && lower_name == "natives" {
            add_score(
                scores,
                signals,
                "re-engine",
                18,
                "RE Engine native assets folder",
                &entry.relative_path,
            );
        }
    }
}

fn detect_game_id(entries: &[ProbeEntry], steam_app_id: Option<&str>) -> Option<String> {
    if let Some(definition) = steam_app_id.and_then(game_definition_by_steam_app_id) {
        return Some(definition.id.clone());
    }

    game_definitions()
        .iter()
        .find(|definition| game_definition_matches(definition, entries))
        .map(|definition| definition.id.clone())
}

fn game_definitions() -> &'static [GameDefinition] {
    static GAME_DEFINITIONS: OnceLock<Vec<GameDefinition>> = OnceLock::new();
    GAME_DEFINITIONS
        .get_or_init(|| {
            parse_json_allow_bom::<Vec<GameDefinition>>(GAME_DEFINITIONS_JSON)
                .expect("bundled UniLoader game definitions must be valid JSON")
        })
        .as_slice()
}

fn game_definition_by_id(game_id: &str) -> Option<&'static GameDefinition> {
    game_definitions()
        .iter()
        .find(|definition| definition.id.eq_ignore_ascii_case(game_id))
}

fn game_definition_by_steam_app_id(app_id: &str) -> Option<&'static GameDefinition> {
    let app_id = app_id.trim();
    if app_id.is_empty() {
        return None;
    }

    game_definitions().iter().find(|definition| {
        definition
            .steam_app_ids
            .iter()
            .any(|known_id| known_id == app_id)
    })
}

fn repair_profile_identity_from_steam_app_id(
    root: &Path,
    profile: &mut GameProfile,
) -> Result<(), String> {
    let Some(definition) = profile
        .steam_app_id
        .as_deref()
        .and_then(game_definition_by_steam_app_id)
    else {
        return Ok(());
    };

    let mut changed = false;
    if profile.game_id.as_deref() != Some(definition.id.as_str()) {
        profile.game_id = Some(definition.id.clone());
        changed = true;
    }
    if let Some(engine) = definition.engine.as_deref() {
        if profile.engine != engine {
            profile.engine = engine.to_string();
            changed = true;
        }
    }
    if profile.loader == "none" {
        if let Some(loader) = definition.bootstrap_runtimes.first() {
            profile.loader = loader.clone();
            changed = true;
        }
    }

    if changed {
        profile.updated_at = now_string();
        let profiles_file = profiles_path(root);
        let mut profiles = read_store::<GameProfile>(&profiles_file).map_err(error_to_string)?;
        let stored_profile = profiles
            .items
            .iter_mut()
            .find(|stored| stored.id == profile.id)
            .ok_or_else(|| format!("Profile not found: {}", profile.id))?;
        *stored_profile = profile.clone();
        write_store(&profiles_file, &profiles).map_err(error_to_string)?;
    }

    Ok(())
}

fn enrich_profile_with_provider_runtime(
    profile: &mut GameProfile,
) -> Option<ProviderRuntimeInference> {
    let inference = infer_profile_foundation_runtime(profile)?;
    let runtime = runtime_definition_by_id(&inference.runtime_id)?;

    profile.loader = runtime.id.clone();
    if profile.engine == "unknown" {
        let mut engines = runtime.profile_engines.clone();
        engines.sort();
        engines.dedup();
        if engines.len() == 1 {
            profile.engine = engines.remove(0);
        }
    }
    profile.updated_at = now_string();

    let entries = walk_game_folder(Path::new(&profile.game_path));
    let _ = prepare_mod_routes(
        Path::new(&profile.game_path),
        profile.game_id.as_deref(),
        &profile.engine,
        &profile.loader,
        &entries,
    );
    Some(inference)
}

fn persist_profile(root: &Path, profile: &GameProfile) -> Result<(), String> {
    let profiles_file = profiles_path(root);
    let mut profiles = read_store::<GameProfile>(&profiles_file).map_err(error_to_string)?;
    let stored_profile = profiles
        .items
        .iter_mut()
        .find(|stored| stored.id == profile.id)
        .ok_or_else(|| format!("Profile not found: {}", profile.id))?;
    *stored_profile = profile.clone();
    write_store(&profiles_file, &profiles).map_err(error_to_string)
}

fn apply_profile_identity_to_detection(
    profile: &GameProfile,
    detection: &mut GameDetectionResult,
    inference: Option<&ProviderRuntimeInference>,
) {
    detection.game_id = profile.game_id.clone();
    detection.engine = profile.engine.clone();
    detection.loader = profile.loader.clone();
    if runtime_definition_by_id(&profile.loader).is_some() {
        detection.recommended_loader = profile.loader.clone();
        detection.loader_installed = runtime_installed(profile, &profile.loader);
        if detection.loader_installed {
            detection.loader_confidence = detection.loader_confidence.max(0.95);
        }
    }

    detection
        .warnings
        .retain(|warning| warning != "Engine could not be identified from this folder.");
    detection
        .warnings
        .retain(|warning| !warning.ends_with(" is recommended but not installed yet."));
    if !detection.loader_installed && detection.recommended_loader != "none" {
        detection.warnings.push(format!(
            "{} is recommended but not installed yet.",
            format_loader(&detection.recommended_loader)
        ));
    }

    if let Some(inference) = inference {
        let provider_label = if inference.providers.is_empty() {
            "provider metadata".to_string()
        } else {
            inference.providers.join(" + ")
        };
        detection.signals.push(DetectionSignal {
            label: format!(
                "{} runtime consensus: {} of {} sampled mods",
                provider_label, inference.supporting_mods, inference.sampled_mods
            ),
            path: inference.runtime_id.clone(),
            weight: 35,
        });
        let ratio = inference.supporting_mods as f64 / inference.sampled_mods.max(1) as f64;
        detection.loader_confidence = detection
            .loader_confidence
            .max((0.6 + ratio * 0.35).min(0.95));
    }

    let entries = walk_game_folder(Path::new(&profile.game_path));
    let preparation = prepare_mod_routes(
        Path::new(&profile.game_path),
        profile.game_id.as_deref(),
        &profile.engine,
        &profile.loader,
        &entries,
    );
    for route in preparation.expected_mod_folders {
        push_unique_route(&mut detection.expected_mod_folders, &route);
    }
    for route in preparation.created_mod_folders {
        push_unique_route(&mut detection.created_mod_folders, &route);
    }
    for warning in preparation.warnings {
        if !detection.warnings.contains(&warning) {
            detection.warnings.push(warning);
        }
    }
}

fn runtime_definitions() -> &'static [RuntimeDefinition] {
    static RUNTIME_DEFINITIONS: OnceLock<Vec<RuntimeDefinition>> = OnceLock::new();
    RUNTIME_DEFINITIONS
        .get_or_init(|| {
            parse_json_allow_bom::<Vec<RuntimeDefinition>>(RUNTIME_DEFINITIONS_JSON)
                .expect("bundled UniLoader runtime definitions must be valid JSON")
        })
        .as_slice()
}

fn runtime_definition_by_id(runtime: &str) -> Option<&'static RuntimeDefinition> {
    runtime_definitions()
        .iter()
        .find(|definition| definition.id.eq_ignore_ascii_case(runtime))
}

fn game_definition_matches(definition: &GameDefinition, entries: &[ProbeEntry]) -> bool {
    entries.iter().any(|entry| {
        let lower_name = entry.name.to_lowercase();
        let lower_path = entry.relative_path.to_lowercase();

        (!entry.is_directory
            && definition
                .executable_names
                .iter()
                .any(|name| lower_name == name.to_lowercase()))
            || definition
                .path_markers
                .iter()
                .map(|marker| marker.to_lowercase())
                .any(|marker| lower_path == marker || lower_path.ends_with(&format!("/{}", marker)))
    })
}

fn score_loaders(
    entries: &[ProbeEntry],
    engine: &str,
    scores: &mut HashMap<String, i32>,
    signals: &mut Vec<DetectionSignal>,
) {
    let bepinex_loader = if engine == "unity-il2cpp" {
        "bepinex-il2cpp"
    } else {
        "bepinex"
    };

    for entry in entries {
        let lower_path = entry.relative_path.to_lowercase();
        let lower_name = entry.name.to_lowercase();

        if lower_path == "bepinex" || lower_path.ends_with("/bepinex") {
            add_score(
                scores,
                signals,
                bepinex_loader,
                8,
                "BepInEx folder",
                &entry.relative_path,
            );
        }

        if !entry.is_directory
            && (lower_path == "bepinex/core/bepinex.dll"
                || lower_path.ends_with("/bepinex/core/bepinex.dll"))
        {
            add_score(
                scores,
                signals,
                bepinex_loader,
                42,
                "BepInEx core DLL",
                &entry.relative_path,
            );
        }

        if !entry.is_directory && lower_name == "doorstop_config.ini" {
            add_score(
                scores,
                signals,
                bepinex_loader,
                16,
                "Doorstop config",
                &entry.relative_path,
            );
        }

        if !entry.is_directory && lower_name == "winhttp.dll" && engine.starts_with("unity") {
            add_score(
                scores,
                signals,
                bepinex_loader,
                16,
                "BepInEx bootstrap DLL",
                &entry.relative_path,
            );
        }

        if lower_path == "bepinex/interop"
            || lower_path.starts_with("bepinex/interop/")
            || lower_path.contains("/bepinex/interop/")
            || lower_path.ends_with("/bepinex/interop")
        {
            add_score(
                scores,
                signals,
                "bepinex-il2cpp",
                28,
                "BepInEx IL2CPP interop folder",
                &entry.relative_path,
            );
        }

        if !entry.is_directory && lower_name == "ue4ss.dll" {
            add_score(
                scores,
                signals,
                "ue4ss",
                44,
                "UE4SS DLL",
                &entry.relative_path,
            );
        }

        if !entry.is_directory && lower_name == "ue4ss-settings.ini" {
            add_score(
                scores,
                signals,
                "ue4ss",
                34,
                "UE4SS settings file",
                &entry.relative_path,
            );
        }

        if lower_path.contains("/binaries/win64/mods")
            || lower_path.contains("/binaries/win64/ue4ss/mods")
            || lower_path.starts_with("mods/")
            || lower_path.starts_with("ue4ss/mods/")
        {
            add_score(
                scores,
                signals,
                "ue4ss",
                18,
                "UE4SS mods folder",
                &entry.relative_path,
            );
        }

        if is_native_script_mod_dir_path(&lower_path)
            || (!entry.is_directory
                && lower_name.ends_with(".as")
                && relative_after_native_script_mods(&entry.relative_path).is_some())
        {
            add_score(
                scores,
                signals,
                "loose-files",
                30,
                "Native script mods folder",
                &entry.relative_path,
            );
        }

        if lower_path == "reframework" || lower_path.ends_with("/reframework") {
            add_score(
                scores,
                signals,
                "reframework",
                8,
                "REFramework folder",
                &entry.relative_path,
            );
        }

        if !entry.is_directory && lower_name == "dinput8.dll" && engine == "re-engine" {
            add_score(
                scores,
                signals,
                "reframework",
                24,
                "REFramework bootstrap DLL",
                &entry.relative_path,
            );
        }
    }
}

fn prepare_mod_routes(
    game_path: &Path,
    game_id: Option<&str>,
    engine: &str,
    loader: &str,
    entries: &[ProbeEntry],
) -> RoutePreparation {
    let mut routes = Vec::new();
    let supported_adapters =
        supported_adapters_for_detection(game_path, game_id, engine, loader, entries);
    let supports = |adapter: &str| {
        supported_adapters
            .iter()
            .any(|supported| supported.eq_ignore_ascii_case(adapter))
    };

    if supports("bepinex") {
        let roots = detected_loader_roots(entries, "bepinex", "BepInEx");
        for root in roots {
            push_unique_route(&mut routes, &format!("{root}/plugins"));
            push_unique_route(&mut routes, &format!("{root}/config"));
        }
    }

    if supports("unreal-pak") {
        for pak_root in find_unreal_pak_roots(game_path) {
            push_unique_route(&mut routes, &format!("{}/~mods", pak_root));
        }
    }

    if supports("ue4ss") {
        let win64_roots = find_unreal_win64_dirs(entries);
        for route in ue4ss_mod_routes_for_entries(entries, &win64_roots) {
            push_unique_route(&mut routes, &route);
        }
    }

    if supports("script-files") {
        for script_root in native_script_routes_for_detection(game_id, entries) {
            push_unique_route(&mut routes, &script_root);
        }
    }

    if supports("reframework") {
        let roots = detected_loader_roots(entries, "reframework", "reframework");
        for root in roots {
            push_unique_route(&mut routes, &format!("{root}/autorun"));
            push_unique_route(&mut routes, &format!("{root}/plugins"));
        }
    }

    let mut preparation = RoutePreparation::default();

    for route in routes {
        preparation.expected_mod_folders.push(route.clone());
        match safe_join(game_path, &route) {
            Ok(path) if path.is_dir() => {}
            Ok(path) if path.exists() => preparation.warnings.push(format!(
                "Expected mod route exists as a file and was not changed: {}.",
                route
            )),
            Ok(path) => {
                if let Err(error) = fs::create_dir_all(&path) {
                    preparation.warnings.push(format!(
                        "Could not create expected mod route {}: {}.",
                        route, error
                    ));
                } else {
                    preparation.created_mod_folders.push(route);
                }
            }
            Err(error) => preparation.warnings.push(format!(
                "Skipped unsafe expected mod route {}: {}.",
                route, error
            )),
        }
    }

    preparation
}

fn detected_loader_roots(entries: &[ProbeEntry], loader_dir: &str, fallback: &str) -> Vec<String> {
    let mut roots = entries
        .iter()
        .filter(|entry| entry.is_directory && entry.name.eq_ignore_ascii_case(loader_dir))
        .map(|entry| entry.relative_path.clone())
        .collect::<Vec<_>>();
    roots.sort();
    roots.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    if roots.is_empty() {
        roots.push(fallback.to_string());
    }
    roots
}

fn push_unique_route(routes: &mut Vec<String>, route: &str) {
    let normalized = normalize_archive_path(route);
    if !routes
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&normalized))
    {
        routes.push(normalized);
    }
}

fn find_unreal_win64_dirs(entries: &[ProbeEntry]) -> Vec<String> {
    let mut roots = entries
        .iter()
        .filter(|entry| {
            let lower_path = entry.relative_path.to_lowercase();
            entry.is_directory
                && (lower_path == "binaries/win64" || lower_path.ends_with("/binaries/win64"))
        })
        .map(|entry| entry.relative_path.clone())
        .collect::<Vec<_>>();

    roots.sort();
    roots.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    roots
}

fn ue4ss_mod_routes_for_entries(entries: &[ProbeEntry], win64_roots: &[String]) -> Vec<String> {
    let mut routes = Vec::new();

    for win64_root in win64_roots {
        let nested_root = format!("{win64_root}/ue4ss");
        let nested_mods = format!("{nested_root}/Mods");
        let legacy_mods = format!("{win64_root}/Mods");
        let has_nested_layout = probe_directory_exists(entries, &nested_root)
            || probe_directory_exists(entries, &nested_mods);

        if has_nested_layout {
            push_unique_route(&mut routes, &nested_mods);
            if probe_route_has_descendants(entries, &legacy_mods) {
                push_unique_route(&mut routes, &legacy_mods);
            }
        } else {
            push_unique_route(&mut routes, &legacy_mods);
        }
    }

    routes
}

fn probe_directory_exists(entries: &[ProbeEntry], route: &str) -> bool {
    entries.iter().any(|entry| {
        entry.is_directory
            && normalize_archive_path(&entry.relative_path).eq_ignore_ascii_case(route)
    })
}

fn probe_route_has_descendants(entries: &[ProbeEntry], route: &str) -> bool {
    let prefix = format!("{}/", normalize_archive_path(route).to_lowercase());
    entries.iter().any(|entry| {
        normalize_archive_path(&entry.relative_path)
            .to_lowercase()
            .starts_with(&prefix)
    })
}

fn native_script_routes_for_detection(
    game_id: Option<&str>,
    entries: &[ProbeEntry],
) -> Vec<String> {
    let mut routes = Vec::new();

    if let Some(definition) = game_id.and_then(game_definition_by_id) {
        for route in &definition.native_script_roots {
            push_unique_route(&mut routes, route);
        }
    }

    for route in find_native_script_routes(entries) {
        push_unique_route(&mut routes, &route);
    }

    routes
}

fn find_native_script_routes(entries: &[ProbeEntry]) -> Vec<String> {
    let mut routes = Vec::new();

    for entry in entries.iter().filter(|entry| entry.is_directory) {
        let lower_path = entry.relative_path.to_lowercase();
        if !is_valid_native_script_route(&lower_path) {
            continue;
        }

        if is_native_script_mod_dir_path(&lower_path) {
            push_unique_route(&mut routes, &entry.relative_path);
        } else if is_native_script_parent_dir_path(&lower_path) {
            push_unique_route(&mut routes, &format!("{}/Mods", entry.relative_path));
        }
    }

    routes
}

fn supports_native_script_mods(game_id: Option<&str>, entries: &[ProbeEntry]) -> bool {
    game_id
        .and_then(game_definition_by_id)
        .map(|definition| !definition.native_script_roots.is_empty())
        .unwrap_or(false)
        || !find_native_script_routes(entries).is_empty()
        || entries.iter().any(|entry| {
            !entry.is_directory
                && entry.name.to_lowercase().ends_with(".as")
                && relative_after_native_script_mods(&entry.relative_path).is_some()
        })
}

fn is_native_script_mod_dir_path(lower_path: &str) -> bool {
    lower_path == "script/mods"
        || lower_path == "scripts/mods"
        || lower_path.ends_with("/script/mods")
        || lower_path.ends_with("/scripts/mods")
}

fn is_native_script_parent_dir_path(lower_path: &str) -> bool {
    lower_path == "script"
        || lower_path == "scripts"
        || lower_path.ends_with("/script")
        || lower_path.ends_with("/scripts")
}

fn is_valid_native_script_route(lower_path: &str) -> bool {
    !lower_path.starts_with("engine/")
        && !lower_path.contains("/engine/")
        && !lower_path.contains("/.git/")
        && !lower_path.contains("/node_modules/")
}

fn scan_import_source(store_root: &Path, source_path: &Path) -> Result<ScannedArchive, String> {
    cleanup_stale_import_cache(store_root);
    if source_path.is_dir() {
        return scan_folder_source(source_path, folder_import_name(source_path));
    }

    if !source_path.is_file() {
        return Err(
            "Import source must be a .zip file, .7z file, .rar file, or folder.".to_string(),
        );
    }

    match source_path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_lowercase())
        .as_deref()
    {
        Some("zip") => scan_zip_archive(source_path),
        Some("7z") => {
            let extracted_dir = extract_7z_to_cache(store_root, source_path)?;
            scan_folder_source(&extracted_dir, folder_import_name(source_path))
        }
        Some("rar") => {
            let extracted_dir = extract_rar_to_cache(store_root, source_path)?;
            scan_folder_source(&extracted_dir, folder_import_name(source_path))
        }
        _ => {
            Err("Only .zip, .7z, .rar, and folder imports are supported in this build.".to_string())
        }
    }
}

fn extract_7z_to_cache(store_root: &Path, archive_path: &Path) -> Result<PathBuf, String> {
    let import_dir = cache_import_dir(store_root, archive_path);
    fs::create_dir_all(&import_dir).map_err(error_to_string)?;
    let mut entry_count = 0_usize;
    let mut expanded_bytes = 0_u64;
    sevenz_rust2::decompress_file_with_extract_fn(
        archive_path,
        &import_dir,
        |entry, reader, destination| {
            entry_count += 1;
            if entry_count > MAX_ARCHIVE_ENTRIES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "7Z archive contains too many entries",
                )
                .into());
            }
            validate_archive_relative_path(&entry.name)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            validate_archive_file_size(&entry.name, entry.size, entry.compressed_size)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            expanded_bytes = expanded_bytes
                .checked_add(entry.size)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "7Z size overflow"))?;
            if expanded_bytes > MAX_ARCHIVE_EXPANDED_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "7Z archive exceeds the expanded-size safety limit",
                )
                .into());
            }
            if entry.is_directory {
                fs::create_dir_all(destination)?;
                return Ok(true);
            }
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut output = File::create(destination)?;
            copy_with_limit(reader, &mut output, MAX_ARCHIVE_FILE_BYTES)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            Ok(true)
        },
    )
    .map_err(error_to_string)?;
    Ok(import_dir)
}

fn extract_rar_to_cache(store_root: &Path, archive_path: &Path) -> Result<PathBuf, String> {
    let import_dir = cache_import_dir(store_root, archive_path);
    fs::create_dir_all(&import_dir).map_err(error_to_string)?;
    let archive = rars::ArchiveReader::read_path(archive_path)
        .map_err(|error| format!("Could not read RAR archive: {}", error))?;
    let extraction_root = import_dir.clone();
    let total_written = Arc::new(AtomicU64::new(0));
    let mut entry_count = 0_usize;

    archive
        .extract_to(None, |meta| {
            entry_count += 1;
            if entry_count > MAX_ARCHIVE_ENTRIES {
                return Err(rars::Error::from(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "RAR archive contains too many entries",
                )));
            }
            let member_name = normalize_archive_path(&meta.name_lossy());
            validate_archive_relative_path(&member_name).map_err(|error| {
                rars::Error::from(io::Error::new(io::ErrorKind::InvalidData, error))
            })?;
            let destination_path = safe_join(&extraction_root, &member_name).map_err(|error| {
                rars::Error::from(io::Error::new(io::ErrorKind::InvalidData, error))
            })?;

            if meta.is_directory {
                fs::create_dir_all(&destination_path)?;
                return Ok(Box::new(io::sink()) as Box<dyn io::Write>);
            }

            if let Some(parent) = destination_path.parent() {
                fs::create_dir_all(parent)?;
            }

            Ok(Box::new(LimitedExtractionWriter {
                file: File::create(destination_path)?,
                file_written: 0,
                total_written: total_written.clone(),
            }) as Box<dyn io::Write>)
        })
        .map_err(|error| format!("Could not extract RAR archive: {}", error))?;

    Ok(import_dir)
}

fn cache_import_dir(store_root: &Path, archive_path: &Path) -> PathBuf {
    store_root.join("cache").join("imports").join(format!(
        "{}-{}",
        sanitize_file_segment(&archive_stem(&folder_import_name(archive_path))),
        Uuid::new_v4()
    ))
}

fn scan_zip_archive(archive_path: &Path) -> Result<ScannedArchive, String> {
    if archive_path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| !extension.eq_ignore_ascii_case("zip"))
        .unwrap_or(true)
    {
        return Err("Only .zip imports are supported in this build.".to_string());
    }

    let file = File::open(archive_path).map_err(error_to_string)?;
    let mut archive = ZipArchive::new(file).map_err(error_to_string)?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(format!(
            "ZIP archive contains too many entries (maximum {}).",
            MAX_ARCHIVE_ENTRIES
        ));
    }
    let mut expanded_bytes = 0_u64;
    let raw_paths = (0..archive.len())
        .filter_map(|index| {
            archive
                .by_index(index)
                .ok()
                .map(|file| normalize_archive_path(file.name()))
        })
        .collect::<Vec<_>>();
    let common_top_folder = common_top_folder(&raw_paths);
    let mut entries = Vec::new();

    for index in 0..archive.len() {
        let file = archive.by_index(index).map_err(error_to_string)?;
        let file_path = normalize_archive_path(file.name());
        validate_archive_relative_path(&file_path)?;
        if !file.is_dir() {
            validate_archive_file_size(&file_path, file.size(), file.compressed_size())?;
            expanded_bytes = expanded_bytes
                .checked_add(file.size())
                .ok_or_else(|| "ZIP expanded size overflowed its safety limit.".to_string())?;
            if expanded_bytes > MAX_ARCHIVE_EXPANDED_BYTES {
                return Err("ZIP archive exceeds the expanded-size safety limit.".to_string());
            }
        }
        entries.push(ArchiveEntry {
            logical_path: to_logical_path(&file_path, common_top_folder.as_deref()),
            path: file_path,
            size: file.size(),
            is_directory: file.is_dir(),
        });
    }

    let (manifest, package_identity) = read_manifest(archive_path, &entries)?;

    Ok(ScannedArchive {
        archive_path: archive_path.to_string_lossy().to_string(),
        archive_name: archive_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("mod.zip")
            .to_string(),
        entries,
        manifest,
        package_identity,
    })
}

fn scan_folder_source(folder_path: &Path, import_name: String) -> Result<ScannedArchive, String> {
    let raw_paths = collect_folder_relative_paths(folder_path)?;
    let common_top_folder = common_top_folder(&raw_paths);
    let mut entries = Vec::new();

    for relative_path in raw_paths {
        let absolute_path = safe_join(folder_path, &relative_path)?;
        let metadata = fs::metadata(&absolute_path).map_err(error_to_string)?;
        entries.push(ArchiveEntry {
            logical_path: to_logical_path(&relative_path, common_top_folder.as_deref()),
            path: relative_path,
            size: if metadata.is_file() {
                metadata.len()
            } else {
                0
            },
            is_directory: metadata.is_dir(),
        });
    }

    let (manifest, package_identity) = read_folder_manifest(folder_path, &entries)?;

    Ok(ScannedArchive {
        archive_path: folder_path.to_string_lossy().to_string(),
        archive_name: import_name,
        entries,
        manifest,
        package_identity,
    })
}

fn collect_folder_relative_paths(folder_path: &Path) -> Result<Vec<String>, String> {
    let mut paths = Vec::new();
    let mut queue = VecDeque::from([(folder_path.to_path_buf(), 0_usize)]);
    let mut total_bytes = 0_u64;

    while let Some((current_path, depth)) = queue.pop_front() {
        if depth > MAX_IMPORT_SCAN_DEPTH {
            return Err(format!(
                "Folder import is nested too deeply (maximum {}).",
                MAX_IMPORT_SCAN_DEPTH
            ));
        }
        let dirents = fs::read_dir(&current_path).map_err(error_to_string)?;
        for dirent in dirents {
            let dirent = dirent.map_err(error_to_string)?;
            let file_type = dirent.file_type().map_err(error_to_string)?;
            if file_type.is_symlink() {
                continue;
            }
            let absolute_path = dirent.path();
            let relative_path = absolute_path
                .strip_prefix(folder_path)
                .map_err(error_to_string)
                .map(to_portable_path)?;
            validate_archive_relative_path(&relative_path)?;
            if file_type.is_dir() {
                queue.push_back((absolute_path, depth + 1));
            } else if file_type.is_file() {
                if paths.len() >= MAX_ARCHIVE_ENTRIES {
                    return Err(format!(
                        "Folder import contains too many files (maximum {}).",
                        MAX_ARCHIVE_ENTRIES
                    ));
                }
                let size = dirent.metadata().map_err(error_to_string)?.len();
                validate_archive_file_size(&relative_path, size, size)?;
                total_bytes = total_bytes
                    .checked_add(size)
                    .ok_or_else(|| "Folder import size overflowed its safety limit.".to_string())?;
                if total_bytes > MAX_ARCHIVE_EXPANDED_BYTES {
                    return Err("Folder import exceeds the total-size safety limit.".to_string());
                }
                paths.push(relative_path);
            }
        }
    }

    Ok(paths)
}

struct LimitedExtractionWriter {
    file: File,
    file_written: u64,
    total_written: Arc<AtomicU64>,
}

impl Write for LimitedExtractionWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let next_file_size = self.file_written.saturating_add(buffer.len() as u64);
        if next_file_size > MAX_ARCHIVE_FILE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "RAR entry exceeds the per-file safety limit",
            ));
        }
        let previous_total = self
            .total_written
            .fetch_add(buffer.len() as u64, Ordering::Relaxed);
        if previous_total.saturating_add(buffer.len() as u64) > MAX_ARCHIVE_EXPANDED_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "RAR archive exceeds the expanded-size safety limit",
            ));
        }
        let written = self.file.write(buffer)?;
        self.file_written = self.file_written.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

fn cleanup_stale_import_cache(store_root: &Path) {
    let cache_root = store_root.join("cache").join("imports");
    let Ok(entries) = fs::read_dir(&cache_root) else {
        return;
    };
    let max_age = Duration::from_secs((IMPORT_CACHE_MAX_AGE_HOURS as u64) * 60 * 60);
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if modified.elapsed().map(|age| age > max_age).unwrap_or(false) {
            let path = entry.path();
            if metadata.is_dir() {
                let _ = fs::remove_dir_all(path);
            } else {
                let _ = fs::remove_file(path);
            }
        }
    }
}

fn analyze_scanned_archive(scanned: ScannedArchive, profile: &GameProfile) -> ArchiveAnalysis {
    analyze_scanned_archive_with_identity(scanned, profile, None)
}

fn analyze_scanned_archive_with_identity(
    scanned: ScannedArchive,
    profile: &GameProfile,
    source_identity: Option<PackageIdentity>,
) -> ArchiveAnalysis {
    let mut plans = vec![
        bepinex_plan(&scanned, profile),
        native_script_plan(&scanned, profile),
        ue4ss_plan(&scanned, profile),
        reframework_plan(&scanned, profile),
        re_engine_native_plan(&scanned, profile),
        unreal_pak_plan(&scanned, profile),
        loose_files_plan(&scanned),
    ]
    .into_iter()
    .flatten()
    .map(|plan| attach_manifest_dependencies(plan, scanned.manifest.as_ref()))
    .collect::<Vec<_>>();

    plans.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut package_identity =
        merge_package_identity(scanned.package_identity.clone(), source_identity);
    for plan in plans.iter().filter(|plan| plan.adapter_id != "loose-files") {
        push_unique_string_value(&mut package_identity.mod_types, &plan.adapter_id);
    }
    if let Some(best_confidence) = plans
        .iter()
        .filter(|plan| plan.adapter_id != "loose-files")
        .map(|plan| plan.confidence)
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
    {
        package_identity.confidence = package_identity.confidence.max(best_confidence);
    }

    let mut first_block_reason = None;
    let recommended_plan = plans.iter().find_map(|plan| {
        if plan.adapter_id == "loose-files" || plan.requires_confirmation {
            return None;
        }
        if let Some(reason) = incompatible_package_reason(profile, plan, Some(&package_identity)) {
            if first_block_reason.is_none() {
                first_block_reason = Some(reason);
            }
            return None;
        }
        Some(plan.clone())
    });
    let supported_mod_types = supported_adapters_for_profile(profile);
    let compatibility = if let Some(plan) = recommended_plan.as_ref() {
        CompatibilityResult {
            status: "compatible".to_string(),
            reason: format!(
                "{} matches the verified {} install route.",
                plan.adapter_name,
                profile_game_label(profile)
            ),
            confidence: plan
                .confidence
                .min(package_identity.confidence.max(plan.confidence)),
            game_id: profile.game_id.clone(),
            provider_game_id: package_identity.provider_game_id.clone(),
            detected_mod_types: package_identity.mod_types.clone(),
            supported_mod_types,
        }
    } else {
        CompatibilityResult {
            status: "blocked".to_string(),
            reason: first_block_reason.unwrap_or_else(|| {
                "UniLoader could not identify a verified automatic install route for this package. No game files were changed."
                    .to_string()
            }),
            confidence: package_identity.confidence,
            game_id: profile.game_id.clone(),
            provider_game_id: package_identity.provider_game_id.clone(),
            detected_mod_types: package_identity.mod_types.clone(),
            supported_mod_types,
        }
    };

    ArchiveAnalysis {
        archive_path: scanned.archive_path,
        archive_name: scanned.archive_name,
        entries: scanned.entries,
        manifest: scanned.manifest,
        package_identity,
        compatibility,
        plans,
        recommended_plan,
    }
}

fn merge_package_identity(
    embedded: Option<PackageIdentity>,
    source: Option<PackageIdentity>,
) -> PackageIdentity {
    let mut identity = source
        .or_else(|| embedded.clone())
        .unwrap_or_else(|| PackageIdentity {
            provider: "unknown".to_string(),
            package_id: None,
            version: None,
            provider_game_id: None,
            mod_types: Vec::new(),
            dependencies: Vec::new(),
            evidence: Vec::new(),
            confidence: 0.0,
        });

    if let Some(embedded) = embedded {
        if identity.provider == "unknown" {
            identity.provider = embedded.provider.clone();
        } else if embedded.provider != "unknown" && embedded.provider != identity.provider {
            push_unique_string_value(
                &mut identity.evidence,
                &format!(
                    "Download source is {}; embedded metadata is {}",
                    identity.provider, embedded.provider
                ),
            );
        }
        if identity.package_id.is_none() {
            identity.package_id = embedded.package_id;
        }
        if identity.version.is_none() {
            identity.version = embedded.version;
        }
        if identity.provider_game_id.is_none() {
            identity.provider_game_id = embedded.provider_game_id;
        }
        for mod_type in embedded.mod_types {
            push_unique_string_value(&mut identity.mod_types, &mod_type);
        }
        for dependency in embedded.dependencies {
            push_unique_string_value(&mut identity.dependencies, &dependency);
        }
        for evidence in embedded.evidence {
            push_unique_string_value(&mut identity.evidence, &evidence);
        }
        identity.confidence = identity.confidence.max(embedded.confidence);
    }

    identity
}

fn push_unique_string_value(values: &mut Vec<String>, value: &str) {
    if !values
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(value))
    {
        values.push(value.to_string());
    }
}

fn provider_source_identity(
    provider: &str,
    package_id: String,
    version: Option<String>,
    provider_game_id: Option<String>,
    evidence: &str,
) -> PackageIdentity {
    PackageIdentity {
        provider: provider.to_string(),
        package_id: Some(package_id),
        version,
        provider_game_id,
        mod_types: Vec::new(),
        dependencies: Vec::new(),
        evidence: vec![evidence.to_string()],
        confidence: 0.98,
    }
}

fn bepinex_plan(scanned: &ScannedArchive, profile: &GameProfile) -> Option<InstallPlan> {
    let files = installable_files(&scanned.entries);
    let mut mappings = Vec::new();
    let mut warnings = Vec::new();
    let target_roots = bepinex_target_roots(profile);

    for file in &files {
        let lower_path = file.logical_path.to_lowercase();
        if let Some(relative_path) = path_after_named_segment(&file.logical_path, "bepinex") {
            for target_root in &target_roots {
                mappings.push(mapping(
                    &file.path,
                    "game",
                    &join_install_route(target_root, &relative_path),
                    "Archive contains a BepInEx folder layout.",
                ));
            }
        } else if let Some(relative_path) =
            path_after_named_segment(&file.logical_path, "doorstop_libs")
        {
            for target_root in &target_roots {
                mappings.push(mapping(
                    &file.path,
                    "game",
                    &join_install_route(
                        &install_route_parent(target_root),
                        &format!("doorstop_libs/{relative_path}"),
                    ),
                    "Doorstop runtime support file.",
                ));
            }
        } else if let Some(relative_path) =
            path_after_named_segment(&file.logical_path, "unstripped_corlib")
        {
            for target_root in &target_roots {
                mappings.push(mapping(
                    &file.path,
                    "game",
                    &join_install_route(
                        &install_route_parent(target_root),
                        &format!("unstripped_corlib/{relative_path}"),
                    ),
                    "BepInEx runtime support file.",
                ));
            }
        } else if let Some(relative_path) = path_after_named_segment(&file.logical_path, "dotnet") {
            for target_root in &target_roots {
                mappings.push(mapping(
                    &file.path,
                    "game",
                    &join_install_route(
                        &install_route_parent(target_root),
                        &format!("dotnet/{relative_path}"),
                    ),
                    "BepInEx bundled runtime file.",
                ));
            }
        } else if is_bepinex_root_runtime_file(&file.logical_path) {
            for target_root in &target_roots {
                mappings.push(mapping(
                    &file.path,
                    "game",
                    &join_install_route(
                        &install_route_parent(target_root),
                        &basename(&file.logical_path),
                    ),
                    "BepInEx bootstrap file.",
                ));
            }
        } else if lower_path.starts_with("plugins/") {
            for target_root in &target_roots {
                mappings.push(mapping(
                    &file.path,
                    "game",
                    &join_install_route(target_root, &file.logical_path),
                    "Plugin folder maps into BepInEx/plugins.",
                ));
            }
        } else if lower_path.starts_with("config/") || lower_path.ends_with(".cfg") {
            let target_suffix = if lower_path.starts_with("config/") {
                file.logical_path.clone()
            } else {
                format!("config/{}", basename(&file.logical_path))
            };
            for target_root in &target_roots {
                mappings.push(mapping(
                    &file.path,
                    "game",
                    &join_install_route(target_root, &target_suffix),
                    "BepInEx config file.",
                ));
            }
        } else if is_probable_bepinex_plugin_dll(&file.logical_path, profile) {
            for target_root in &target_roots {
                mappings.push(mapping(
                    &file.path,
                    "game",
                    &join_install_route(
                        target_root,
                        &format!("plugins/{}", basename(&file.logical_path)),
                    ),
                    "Managed plugin DLL.",
                ));
            }
        }
    }

    let has_signal = files.iter().any(|file| {
        let lower_path = file.logical_path.to_lowercase();
        path_after_named_segment(&file.logical_path, "bepinex").is_some()
            || lower_path.starts_with("plugins/")
            || is_probable_bepinex_plugin_dll(&file.logical_path, profile)
            || is_bepinex_root_runtime_file(&file.logical_path)
    });

    if !has_signal || mappings.is_empty() {
        return None;
    }

    let runtime = if profile.engine == "unity-il2cpp" || profile.loader == "bepinex-il2cpp" {
        "bepinex-il2cpp"
    } else {
        "bepinex"
    };

    if profile.engine == "unknown"
        && files
            .iter()
            .any(|file| file.logical_path.to_lowercase().ends_with(".dll"))
    {
        warnings
            .push("This looks like a BepInEx mod, but the profile engine is unknown.".to_string());
    }

    Some(InstallPlan {
        adapter_id: "bepinex".to_string(),
        adapter_name: "BepInEx / Thunderstore".to_string(),
        confidence: if scanned.manifest.is_some() || profile.loader.contains("bepinex") {
            0.92
        } else {
            0.78
        },
        summary: format!(
            "Install {} file(s) into {}.",
            mappings.len(),
            join_human_list(&target_roots)
        ),
        mappings,
        dependencies: vec![known_runtime_dependency(profile, runtime)],
        warnings,
        requires_confirmation: false,
    })
}

fn native_script_plan(scanned: &ScannedArchive, profile: &GameProfile) -> Option<InstallPlan> {
    let files = installable_files(&scanned.entries);
    let script_files = files
        .iter()
        .filter(|file| file.logical_path.to_lowercase().ends_with(".as"))
        .collect::<Vec<_>>();

    if script_files.is_empty() {
        return None;
    }

    let target_dirs = native_script_target_dirs(profile);
    if target_dirs.is_empty() {
        return None;
    }

    let mut mappings = Vec::new();
    for file in script_files {
        let payload_relative_path = native_script_payload_relative(&file.logical_path);
        for target_dir in &target_dirs {
            mappings.push(mapping(
                &file.path,
                "game",
                &format!("{}/{}", target_dir, payload_relative_path),
                "Native game script mod file.",
            ));
        }
    }

    let mut warnings = Vec::new();
    if profile.engine != "unreal" && profile.engine != "unknown" {
        warnings.push(
            "These look like native script mods, but the selected profile is not marked as Unreal."
                .to_string(),
        );
    }

    Some(InstallPlan {
        adapter_id: "script-files".to_string(),
        adapter_name: "Native Script Mods".to_string(),
        confidence: if profile.engine == "unreal" {
            0.9
        } else {
            0.68
        },
        summary: format!(
            "Install {} script file(s) into {}.",
            mappings.len(),
            join_human_list(&target_dirs)
        ),
        mappings,
        dependencies: Vec::new(),
        warnings,
        requires_confirmation: false,
    })
}

fn ue4ss_plan(scanned: &ScannedArchive, profile: &GameProfile) -> Option<InstallPlan> {
    let files = installable_files(&scanned.entries);
    let mut mappings = Vec::new();
    let mut warnings = Vec::new();
    let mut summary_targets = Vec::new();
    let mod_folder_name = archive_stem(&scanned.archive_name);
    let (target_roots, mod_target_dirs) = ue4ss_install_targets(profile);

    for file in &files {
        let lower_path = file.logical_path.to_lowercase();
        if is_ue4ss_root_runtime_file(&file.logical_path) {
            for target_root in &target_roots {
                push_unique_route(&mut summary_targets, target_root);
                mappings.push(mapping(
                    &file.path,
                    "game",
                    &format!("{target_root}/{}", basename(&file.logical_path)),
                    "UE4SS runtime bootstrap file.",
                ));
            }
        } else if let Some(target_suffix) =
            strip_prefix_ignore_ascii_case(&file.logical_path, "ue4ss/mods/")
                .or_else(|| strip_prefix_ignore_ascii_case(&file.logical_path, "mods/"))
        {
            for target_dir in &mod_target_dirs {
                push_unique_route(&mut summary_targets, target_dir);
                mappings.push(mapping(
                    &file.path,
                    "game",
                    &format!("{target_dir}/{target_suffix}"),
                    "UE4SS Mods folder.",
                ));
            }
        } else if lower_path.starts_with("ue4ss/") {
            for target_root in &target_roots {
                push_unique_route(&mut summary_targets, target_root);
                mappings.push(mapping(
                    &file.path,
                    "game",
                    &format!("{target_root}/{}", file.logical_path),
                    "UE4SS runtime or configuration files.",
                ));
            }
        } else if lower_path.contains("/scripts/") || lower_path.ends_with(".lua") {
            for target_dir in &mod_target_dirs {
                push_unique_route(&mut summary_targets, target_dir);
                mappings.push(mapping(
                    &file.path,
                    "game",
                    &format!("{target_dir}/{mod_folder_name}/{}", file.logical_path),
                    "UE4SS script file.",
                ));
            }
        }
    }

    let has_signal = files.iter().any(|file| {
        let lower_path = file.logical_path.to_lowercase();
        lower_path.starts_with("mods/")
            || lower_path.starts_with("ue4ss/")
            || is_ue4ss_root_runtime_file(&file.logical_path)
            || lower_path.contains("/scripts/")
            || lower_path.ends_with(".lua")
    });

    if !has_signal || mappings.is_empty() {
        return None;
    }

    if profile.engine != "unreal" && profile.engine != "unknown" {
        warnings.push(
            "This looks like a UE4SS mod, but the selected profile is not marked as Unreal."
                .to_string(),
        );
    }

    if files
        .iter()
        .any(|file| file.logical_path.to_lowercase().ends_with(".pak"))
    {
        warnings.push(
            "This archive also contains pak files; the Unreal pak adapter may be a better fit."
                .to_string(),
        );
    }

    Some(InstallPlan {
        adapter_id: "ue4ss".to_string(),
        adapter_name: "UE4SS / Unreal Scripts".to_string(),
        confidence: if profile.loader == "ue4ss" { 0.9 } else { 0.72 },
        summary: format!(
            "Install {} file(s) into {}.",
            mappings.len(),
            join_human_list(&summary_targets)
        ),
        mappings,
        dependencies: vec![known_runtime_dependency(profile, "ue4ss")],
        warnings,
        requires_confirmation: false,
    })
}

fn reframework_plan(scanned: &ScannedArchive, profile: &GameProfile) -> Option<InstallPlan> {
    let files = installable_files(&scanned.entries);
    let mut mappings = Vec::new();
    let mut warnings = Vec::new();
    let target_roots = reframework_target_roots(profile);

    for file in &files {
        let lower_path = file.logical_path.to_lowercase();
        if is_reframework_root_runtime_file(&file.logical_path) {
            for target_root in &target_roots {
                mappings.push(mapping(
                    &file.path,
                    "game",
                    &join_install_route(
                        &install_route_parent(target_root),
                        &basename(&file.logical_path),
                    ),
                    "REFramework bootstrap/runtime file.",
                ));
            }
        } else if let Some(relative_path) =
            path_after_named_segment(&file.logical_path, "reframework")
        {
            for target_root in &target_roots {
                mappings.push(mapping(
                    &file.path,
                    "game",
                    &join_install_route(target_root, &relative_path),
                    "Archive already contains an REFramework folder layout.",
                ));
            }
        } else if lower_path.ends_with(".lua") {
            for target_root in &target_roots {
                mappings.push(mapping(
                    &file.path,
                    "game",
                    &join_install_route(
                        target_root,
                        &format!("autorun/{}", basename(&file.logical_path)),
                    ),
                    "REFramework autorun Lua script.",
                ));
            }
        } else if lower_path.ends_with(".dll") {
            for target_root in &target_roots {
                mappings.push(mapping(
                    &file.path,
                    "game",
                    &join_install_route(
                        target_root,
                        &format!("plugins/{}", basename(&file.logical_path)),
                    ),
                    "REFramework native plugin.",
                ));
            }
        }
    }

    let has_signal = files.iter().any(|file| {
        let lower_path = file.logical_path.to_lowercase();
        path_after_named_segment(&file.logical_path, "reframework").is_some()
            || lower_path.ends_with(".lua")
            || is_reframework_root_runtime_file(&file.logical_path)
    });

    if !has_signal || mappings.is_empty() {
        return None;
    }

    if profile.engine != "re-engine" && profile.engine != "unknown" {
        warnings.push("This looks like an REFramework mod, but the selected profile is not marked as RE Engine.".to_string());
    }

    Some(InstallPlan {
        adapter_id: "reframework".to_string(),
        adapter_name: "REFramework / RE Engine".to_string(),
        confidence: if profile.loader == "reframework" {
            0.9
        } else {
            0.76
        },
        summary: format!(
            "Install {} file(s) into {}.",
            mappings.len(),
            join_human_list(&target_roots)
        ),
        mappings,
        dependencies: vec![known_runtime_dependency(profile, "reframework")],
        warnings,
        requires_confirmation: false,
    })
}

fn re_engine_native_plan(scanned: &ScannedArchive, profile: &GameProfile) -> Option<InstallPlan> {
    let mappings = installable_files(&scanned.entries)
        .into_iter()
        .filter_map(|file| {
            let target = re_engine_native_relative_path(&file.logical_path)?;
            Some(mapping(
                &file.path,
                "game",
                &target,
                "Preserve the verified RE Engine natives asset layout.",
            ))
        })
        .collect::<Vec<_>>();

    if mappings.is_empty() {
        return None;
    }

    Some(InstallPlan {
        adapter_id: "re-engine-native".to_string(),
        adapter_name: "RE Engine Native Assets".to_string(),
        confidence: if profile.engine == "re-engine" {
            0.92
        } else {
            0.72
        },
        summary: format!(
            "Install {} file(s) while preserving the RE Engine natives layout.",
            mappings.len()
        ),
        mappings,
        dependencies: Vec::new(),
        warnings: Vec::new(),
        requires_confirmation: false,
    })
}

fn re_engine_native_relative_path(path: &str) -> Option<String> {
    let normalized = normalize_archive_path(path);
    let parts = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let natives_index = parts
        .iter()
        .position(|part| part.eq_ignore_ascii_case("natives"))?;
    if natives_index + 1 >= parts.len() {
        return None;
    }

    Some(parts[natives_index..].join("/"))
}

fn unreal_pak_plan(scanned: &ScannedArchive, profile: &GameProfile) -> Option<InstallPlan> {
    let pak_files = installable_files(&scanned.entries)
        .into_iter()
        .filter(|file| {
            let lower_path = file.logical_path.to_lowercase();
            lower_path.ends_with(".pak")
                || lower_path.ends_with(".ucas")
                || lower_path.ends_with(".utoc")
        })
        .collect::<Vec<_>>();

    if pak_files.is_empty() {
        return None;
    }

    let pak_target_dirs = unreal_pak_target_dirs(profile);
    let mappings = pak_files
        .iter()
        .flat_map(|file| {
            pak_target_dirs.iter().map(|pak_target_dir| {
                mapping(
                    &file.path,
                    "game",
                    &format!("{}/{}", pak_target_dir, basename(&file.logical_path)),
                    "Generic Unreal Engine pak-style mod file.",
                )
            })
        })
        .collect::<Vec<_>>();
    let warnings = if profile.engine == "unreal" || profile.engine == "unknown" {
        Vec::new()
    } else {
        vec![
            "Pak mods are normally Unreal Engine mods; verify this profile before installing."
                .to_string(),
        ]
    };

    Some(InstallPlan {
        adapter_id: "unreal-pak".to_string(),
        adapter_name: "Unreal Pak Files".to_string(),
        confidence: if profile.engine == "unreal" {
            0.88
        } else {
            0.66
        },
        summary: format!(
            "Deploy {} pak file(s) to {}.",
            pak_files.len(),
            join_human_list(&pak_target_dirs)
        ),
        mappings,
        dependencies: Vec::new(),
        warnings,
        requires_confirmation: false,
    })
}

fn loose_files_plan(scanned: &ScannedArchive) -> Option<InstallPlan> {
    let files = installable_files(&scanned.entries);
    if files.is_empty() {
        return None;
    }

    let mappings = files
        .iter()
        .map(|file| {
            mapping(
                &file.path,
                "profile",
                &format!("staged/{}", file.logical_path),
                "Unrecognized loose file staged in the profile instead of copied into the game.",
            )
        })
        .collect::<Vec<_>>();

    Some(InstallPlan {
        adapter_id: "loose-files".to_string(),
        adapter_name: "Loose Files".to_string(),
        confidence: 0.25,
        summary: format!(
            "Stage {} unrecognized file(s) for manual review.",
            mappings.len()
        ),
        mappings,
        dependencies: Vec::new(),
        warnings: vec![
            "UniLoader could not identify a safe game-specific install layout.".to_string(),
            "Files will be staged in the profile data folder instead of deployed into the game."
                .to_string(),
        ],
        requires_confirmation: true,
    })
}

#[derive(Debug, Clone, Default)]
struct InstallMetadata {
    archive_name: Option<String>,
    package_id: Option<String>,
    dependency_string: Option<String>,
    display_name: Option<String>,
    package_identity: Option<PackageIdentity>,
    runtime_id: Option<String>,
    icon_url: Option<String>,
}

struct InstallOptions<'a> {
    metadata: InstallMetadata,
    resolve_dependencies: bool,
    visited_dependencies: &'a mut HashSet<String>,
    dependency_depth: usize,
}

#[derive(Debug)]
struct DeploymentRollbackEntry {
    destination: PathBuf,
    immediate_backup: Option<PathBuf>,
}

#[derive(Debug)]
struct DeploymentOutcome {
    files_written: Vec<String>,
    backups_written: Vec<String>,
    written_file_hashes: HashMap<String, String>,
    transaction_root: PathBuf,
    rollback_entries: Vec<DeploymentRollbackEntry>,
}

impl DeploymentOutcome {
    fn commit(self) {
        let _ = fs::remove_dir_all(self.transaction_root);
    }

    fn rollback(&self) {
        for entry in self.rollback_entries.iter().rev() {
            if let Some(backup) = &entry.immediate_backup {
                if let Some(parent) = entry.destination.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let _ = replace_file_from_path(backup, &entry.destination);
            } else if entry.destination.exists() {
                let _ = fs::remove_file(&entry.destination);
            }
        }
        let _ = fs::remove_dir_all(&self.transaction_root);
    }
}

fn install_archive_impl(
    store_root: &Path,
    profile: &GameProfile,
    archive_path: &str,
    archive_name: Option<&str>,
    package_identity: Option<PackageIdentity>,
    plan: &InstallPlan,
) -> Result<InstallResult, String> {
    let mut visited_dependencies = HashSet::new();
    install_archive_impl_with_metadata(
        store_root,
        profile,
        archive_path,
        plan,
        InstallOptions {
            metadata: InstallMetadata {
                archive_name: archive_name.map(str::to_string),
                package_identity,
                ..InstallMetadata::default()
            },
            resolve_dependencies: true,
            visited_dependencies: &mut visited_dependencies,
            dependency_depth: 0,
        },
    )
}

fn prepare_install_source_for_deployment(
    store_root: &Path,
    source_path: &Path,
) -> Result<PathBuf, String> {
    if source_path.is_dir() {
        return Ok(source_path.to_path_buf());
    }
    if !source_path.is_file() {
        return Err(format!(
            "Import source no longer exists: {}",
            source_path.to_string_lossy()
        ));
    }

    match source_path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("zip") => Ok(source_path.to_path_buf()),
        Some("7z" | "rar") => scan_import_source(store_root, source_path)
            .map(|scanned| PathBuf::from(scanned.archive_path)),
        _ => {
            Err("Only .zip, .7z, .rar, and folder imports are supported in this build.".to_string())
        }
    }
}

fn install_archive_impl_with_metadata(
    store_root: &Path,
    profile: &GameProfile,
    archive_path: &str,
    plan: &InstallPlan,
    options: InstallOptions<'_>,
) -> Result<InstallResult, String> {
    let InstallOptions {
        mut metadata,
        resolve_dependencies,
        visited_dependencies,
        dependency_depth,
    } = options;

    let supplied_runtime = metadata
        .runtime_id
        .clone()
        .or_else(|| runtime_supplied_by_plan(profile, plan));
    let mut effective_plan = plan.clone();
    if let Some(runtime) = supplied_runtime {
        metadata.runtime_id.get_or_insert_with(|| runtime.clone());
        effective_plan.dependencies.retain(|dependency| {
            runtime_id_for_dependency(profile, dependency)
                .map(|required| !required.eq_ignore_ascii_case(&runtime))
                .unwrap_or(true)
        });
    }
    let plan = &effective_plan;

    if dependency_depth > MAX_DEPENDENCY_DEPTH {
        return Err("Dependency chain is too deep to install safely.".to_string());
    }
    if let Some(reason) =
        incompatible_package_reason(profile, plan, metadata.package_identity.as_ref())
    {
        return Err(reason);
    }
    let original_source_path = Path::new(archive_path);
    let prepared_source_path =
        prepare_install_source_for_deployment(store_root, original_source_path)?;
    if let Some(reason) =
        duplicate_installed_mod_reason(store_root, profile, plan, archive_path, &metadata)?
    {
        return Err(reason);
    }

    let install_id = Uuid::new_v4().to_string();
    let installed_at = now_string();
    let managed_source_path =
        materialize_import_source(store_root, profile, &install_id, &prepared_source_path)
            .map_err(|error| format!("Could not stage the mod package: {error}"))?;
    let mut warnings = plan.warnings.clone();

    if resolve_dependencies {
        warnings.extend(install_dependencies_for_plan(
            store_root,
            profile,
            plan,
            visited_dependencies,
            dependency_depth,
        )?);
    }

    let deployment = match deploy_plan_transaction(
        store_root,
        profile,
        &install_id,
        &managed_source_path,
        plan,
        None,
    ) {
        Ok(deployment) => deployment,
        Err(error) => {
            cleanup_install_data(store_root, &profile.id, &install_id);
            return Err(error);
        }
    };

    let finalize = (|| -> Result<InstallResult, String> {
        let dependencies = plan
            .dependencies
            .iter()
            .map(|dependency| refresh_dependency_status(store_root, profile, dependency))
            .collect::<Vec<_>>();
        let archive_name = metadata.archive_name.clone().unwrap_or_else(|| {
            original_source_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("mod.zip")
                .to_string()
        });
        let managed_archive_path = managed_source_path.to_string_lossy().to_string();
        let display_name = metadata
            .runtime_id
            .as_deref()
            .map(|runtime| format_loader(runtime).to_string())
            .unwrap_or_else(|| {
                install_display_name(store_root, &managed_archive_path, plan, &metadata)
            });
        let package_id = metadata.package_id.clone().or_else(|| {
            metadata
                .package_identity
                .as_ref()
                .and_then(|identity| identity.package_id.clone())
        });

        let record = InstalledModRecord {
            id: install_id.clone(),
            profile_id: profile.id.clone(),
            archive_path: managed_archive_path.clone(),
            archive_name,
            display_name: Some(display_name),
            package_id,
            dependency_string: metadata.dependency_string,
            icon_url: metadata.icon_url,
            adapter_id: plan.adapter_id.clone(),
            summary: plan.summary.clone(),
            installed_at: installed_at.clone(),
            files_written: deployment.files_written.clone(),
            backups_written: deployment.backups_written.clone(),
            written_file_hashes: deployment.written_file_hashes.clone(),
            dependencies,
            config_files: config_files_from_paths(&deployment.files_written),
            runtime_id: metadata.runtime_id,
            externally_managed: false,
            enabled: true,
            last_status: "installed".to_string(),
            plan: Some(plan.clone()),
        };

        write_receipt(store_root, profile, &record)
            .map_err(|error| format!("Could not save the install receipt: {error}"))?;
        if let Err(error) = add_installed_mod(store_root, record) {
            let receipt_path = profile_dir(store_root, &profile.id)
                .join("receipts")
                .join(format!("{}.json", install_id));
            let _ = fs::remove_file(receipt_path);
            return Err(format!("Could not save the installed-mod record: {error}"));
        }

        Ok(InstallResult {
            profile_id: profile.id.clone(),
            archive_path: managed_archive_path,
            installed_mod_id: install_id.clone(),
            installed_at: installed_at.clone(),
            files_written: deployment.files_written.clone(),
            backups_written: deployment.backups_written.clone(),
            warnings,
        })
    })();

    match finalize {
        Ok(result) => {
            deployment.commit();
            Ok(result)
        }
        Err(error) => {
            deployment.rollback();
            cleanup_install_data(store_root, &profile.id, &install_id);
            Err(error)
        }
    }
}

fn attach_manifest_dependencies(
    mut plan: InstallPlan,
    manifest: Option<&ThunderstoreManifest>,
) -> InstallPlan {
    if let Some(manifest) = manifest {
        if let Some(dependencies) = &manifest.dependencies {
            for dependency in dependencies {
                let parsed = parse_thunderstore_dependency(dependency);
                if !plan.dependencies.iter().any(|item| item.id == parsed.id) {
                    plan.dependencies.push(parsed);
                }
            }
        }
    }

    plan
}

fn incompatible_package_reason(
    profile: &GameProfile,
    plan: &InstallPlan,
    identity: Option<&PackageIdentity>,
) -> Option<String> {
    if let Some(identity) = identity {
        if let Some(reason) = incompatible_provider_game_reason(profile, identity) {
            return Some(reason);
        }
    }
    incompatible_install_plan_reason(profile, plan)
}

fn incompatible_provider_game_reason(
    profile: &GameProfile,
    identity: &PackageIdentity,
) -> Option<String> {
    let actual = identity.provider_game_id.as_deref()?.trim();
    if actual.is_empty() {
        return None;
    }
    let definition = profile.game_id.as_deref().and_then(game_definition_by_id)?;
    let expected = match identity.provider.as_str() {
        "thunderstore" => definition.thunderstore_community.as_deref(),
        "nexus" => definition.nexus_game_domain.as_deref(),
        "curseforge" => definition.curseforge_game_id.as_deref(),
        _ => None,
    };

    if let Some(expected) = expected {
        if compact_provider_slug(actual) != compact_provider_slug(expected) {
            return Some(format!(
                "This {} package belongs to provider game '{}', but the selected profile is {}. No files were installed.",
                provider_label(&identity.provider),
                actual,
                definition.display_name
            ));
        }
    }
    None
}

fn incompatible_install_plan_reason(profile: &GameProfile, plan: &InstallPlan) -> Option<String> {
    let engine_reason = match plan.adapter_id.as_str() {
        "unreal-pak" | "ue4ss" if profile.engine != "unreal" => Some(format!(
            "{} mods are for Unreal Engine games, but this profile is detected as {}.",
            plan.adapter_name,
            engine_label(&profile.engine)
        )),
        "script-files" if profile.engine != "unreal" && profile.engine != "unknown" => {
            Some(format!(
                "{} are for native script-capable games, but this profile is detected as {}.",
                plan.adapter_name,
                engine_label(&profile.engine)
            ))
        }
        "reframework" if profile.engine != "re-engine" => Some(format!(
            "REFramework mods are for RE Engine games, but this profile is detected as {}.",
            engine_label(&profile.engine)
        )),
        "re-engine-native" if profile.engine != "re-engine" => Some(format!(
            "RE Engine native asset mods are for RE Engine games, but this profile is detected as {}.",
            engine_label(&profile.engine)
        )),
        "bepinex" if !profile.engine.starts_with("unity") && !profile.loader.starts_with("bepinex") => {
            Some(format!(
                "BepInEx mods are for Unity/BepInEx games, but this profile is detected as {} with {}.",
                engine_label(&profile.engine),
                format_loader(&profile.loader)
            ))
        }
        _ => None,
    };
    if engine_reason.is_some() {
        return engine_reason;
    }

    if plan.adapter_id == "loose-files" || plan.requires_confirmation {
        return Some(
            "UniLoader could not identify a safe game-specific install layout, so the package was not deployed."
                .to_string(),
        );
    }

    let supported_adapters = supported_adapters_for_profile(profile);
    if supported_adapters.is_empty() {
        return Some(format!(
            "UniLoader could not prove a safe mod installation route from {}'s folder yet.",
            profile_game_label(profile)
        ));
    }
    if !supported_adapters
        .iter()
        .any(|adapter| adapter.eq_ignore_ascii_case(&plan.adapter_id))
    {
        let supported = supported_adapters
            .iter()
            .map(|adapter| adapter_display_name(adapter).to_string())
            .collect::<Vec<_>>();
        return Some(format!(
            "This is a {}, but {} only supports {}. No files were installed.",
            plan.adapter_name,
            profile_game_label(profile),
            join_human_list(&supported)
        ));
    }

    None
}

fn supported_adapters_for_profile(profile: &GameProfile) -> Vec<String> {
    let game_path = Path::new(&profile.game_path);
    let entries = if profile
        .game_id
        .as_deref()
        .and_then(game_definition_by_id)
        .is_some()
    {
        Vec::new()
    } else {
        walk_game_folder(game_path)
    };
    supported_adapters_for_detection(
        game_path,
        profile.game_id.as_deref(),
        &profile.engine,
        &profile.loader,
        &entries,
    )
}

fn supported_adapters_for_detection(
    game_path: &Path,
    game_id: Option<&str>,
    engine: &str,
    loader: &str,
    entries: &[ProbeEntry],
) -> Vec<String> {
    let mut adapters = game_id
        .and_then(game_definition_by_id)
        .map(|definition| definition.supported_adapters.clone())
        .unwrap_or_default();
    if engine.starts_with("unity") || loader.starts_with("bepinex") {
        push_unique_string_value(&mut adapters, "bepinex");
    }
    if !find_unreal_pak_roots(game_path).is_empty() {
        push_unique_string_value(&mut adapters, "unreal-pak");
    }
    if supports_native_script_mods(game_id, entries) {
        push_unique_string_value(&mut adapters, "script-files");
    }
    if engine == "unreal" && loader == "ue4ss" && !find_unreal_win64_dirs(entries).is_empty() {
        push_unique_string_value(&mut adapters, "ue4ss");
    }
    if engine == "re-engine" || loader == "reframework" {
        push_unique_string_value(&mut adapters, "reframework");
        push_unique_string_value(&mut adapters, "re-engine-native");
    }
    adapters
}

fn profile_game_label(profile: &GameProfile) -> String {
    profile
        .game_id
        .as_deref()
        .and_then(game_definition_by_id)
        .map(|definition| definition.display_name.clone())
        .unwrap_or_else(|| profile.name.clone())
}

fn adapter_display_name(adapter: &str) -> &str {
    match adapter {
        "bepinex" => "BepInEx mods",
        "script-files" => "native script mods",
        "ue4ss" => "UE4SS mods",
        "reframework" => "REFramework mods",
        "re-engine-native" => "RE Engine native asset mods",
        "unreal-pak" => "Unreal Pak mods",
        _ => adapter,
    }
}

fn provider_label(provider: &str) -> &str {
    match provider {
        "thunderstore" => "Thunderstore",
        "nexus" => "Nexus Mods",
        "curseforge" => "CurseForge",
        _ => provider,
    }
}

fn duplicate_installed_mod_reason(
    store_root: &Path,
    profile: &GameProfile,
    plan: &InstallPlan,
    archive_path: &str,
    metadata: &InstallMetadata,
) -> Result<Option<String>, String> {
    let candidate_keys = install_identity_keys(plan, archive_path, metadata);
    if candidate_keys.is_empty() {
        return Ok(None);
    }

    let store = read_store::<InstalledModRecord>(&installed_mods_path(store_root))
        .map_err(error_to_string)?;
    for record in store
        .items
        .iter()
        .filter(|record| record.profile_id == profile.id && record.last_status != "removed")
    {
        let existing_keys = installed_mod_identity_keys(record);
        if candidate_keys
            .iter()
            .any(|candidate| existing_keys.contains(candidate))
        {
            return Ok(Some(format!(
                "{} is already installed in this profile. Disable, enable, or remove the existing copy before importing it again.",
                display_record_name(record)
            )));
        }
    }

    Ok(None)
}

fn install_identity_keys(
    plan: &InstallPlan,
    archive_path: &str,
    metadata: &InstallMetadata,
) -> HashSet<String> {
    let mut keys = HashSet::new();

    if let Some(package_id) = metadata.package_id.as_deref() {
        push_identity_key(&mut keys, "package", package_id);
    }
    if let Some(dependency_string) = metadata.dependency_string.as_deref() {
        push_identity_key(&mut keys, "dependency", dependency_string);
    }
    if let Some(display_name) = metadata.display_name.as_deref() {
        push_mod_name_identity_key(&mut keys, plan, display_name);
    }
    if let Some(primary_source) = primary_mapping_source(plan) {
        push_mod_name_identity_key(&mut keys, plan, &primary_source);
    }
    if let Some(archive_name) = metadata.archive_name.as_deref() {
        push_mod_name_identity_key(&mut keys, plan, archive_name);
    }
    if let Some(file_name) = Path::new(archive_path)
        .file_name()
        .and_then(|name| name.to_str())
    {
        push_mod_name_identity_key(&mut keys, plan, file_name);
    }

    keys
}

fn installed_mod_identity_keys(record: &InstalledModRecord) -> HashSet<String> {
    let mut keys = HashSet::new();

    if let Some(package_id) = record.package_id.as_deref() {
        push_identity_key(&mut keys, "package", package_id);
    }
    if let Some(dependency_string) = record.dependency_string.as_deref() {
        push_identity_key(&mut keys, "dependency", dependency_string);
    }
    if let Some(display_name) = record.display_name.as_deref() {
        push_mod_name_identity_key_for_adapter(&mut keys, &record.adapter_id, display_name);
    }
    push_mod_name_identity_key_for_adapter(&mut keys, &record.adapter_id, &record.archive_name);

    keys
}

fn push_mod_name_identity_key(keys: &mut HashSet<String>, plan: &InstallPlan, value: &str) {
    push_mod_name_identity_key_for_adapter(keys, &plan.adapter_id, value);
}

fn push_mod_name_identity_key_for_adapter(
    keys: &mut HashSet<String>,
    adapter_id: &str,
    value: &str,
) {
    let identity = normalize_mod_identity(value);
    if !identity.is_empty() {
        keys.insert(format!("name:{}:{}", adapter_id.to_lowercase(), identity));
    }
}

fn push_identity_key(keys: &mut HashSet<String>, prefix: &str, value: &str) {
    let normalized = value.trim().to_lowercase();
    if !normalized.is_empty() {
        keys.insert(format!("{prefix}:{normalized}"));
    }
}

fn normalize_mod_identity(value: &str) -> String {
    humanize_mod_display_name(value)
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(|character| character.to_lowercase())
        .collect()
}

fn engine_label(engine: &str) -> &'static str {
    match engine {
        "unity-mono" => "Unity Mono",
        "unity-il2cpp" => "Unity IL2CPP",
        "unreal" => "Unreal Engine",
        "re-engine" => "RE Engine",
        _ => "an unknown engine",
    }
}

fn known_runtime_dependency(profile: &GameProfile, runtime: &str) -> DependencySpec {
    let already_installed = runtime_installed(profile, runtime);
    let definition = runtime_dependency_definition(profile, runtime);

    DependencySpec {
        id: definition.id,
        name: definition.name,
        version: None,
        provider: definition.provider,
        required: true,
        status: if already_installed {
            "already-installed"
        } else {
            "missing"
        }
        .to_string(),
        source: Some(definition.source),
        notes: definition.notes,
    }
}

fn runtime_dependency_definition(
    profile: &GameProfile,
    runtime: &str,
) -> RuntimeDependencyDefinition {
    if let Some(definition) = profile
        .game_id
        .as_deref()
        .and_then(game_definition_by_id)
        .and_then(|game| game.runtime_dependencies.get(runtime))
    {
        return definition.clone();
    }

    if let Some(definition) = runtime_definition_by_id(runtime) {
        return definition.dependency.clone();
    }

    RuntimeDependencyDefinition {
        id: format!("runtime:{runtime}"),
        name: humanize_mod_display_name(runtime),
        provider: "manual".to_string(),
        source: format!("runtime:{runtime}"),
        notes: Some(
            "This runtime needs a registry definition before UniLoader can install it automatically."
                .to_string(),
        ),
    }
}

fn profile_bootstrap_dependencies(profile: &GameProfile) -> Vec<DependencySpec> {
    if let Some(definition) = profile.game_id.as_deref().and_then(game_definition_by_id) {
        if !definition.bootstrap_runtimes.is_empty() {
            return definition
                .bootstrap_runtimes
                .iter()
                .map(|runtime| known_runtime_dependency(profile, runtime))
                .collect();
        }
    }

    match (
        profile.game_id.as_deref(),
        profile.engine.as_str(),
        profile.loader.as_str(),
    ) {
        (Some("valheim"), _, _) => vec![known_runtime_dependency(profile, "bepinex")],
        (_, "unity-mono", "bepinex") => vec![known_runtime_dependency(profile, "bepinex")],
        (_, "unity-il2cpp", "bepinex-il2cpp") => {
            vec![known_runtime_dependency(profile, "bepinex-il2cpp")]
        }
        (_, "unreal", "ue4ss") => vec![known_runtime_dependency(profile, "ue4ss")],
        (_, "re-engine", "reframework") => vec![known_runtime_dependency(profile, "reframework")],
        _ => Vec::new(),
    }
}

fn profile_runtime_ids(profile: &GameProfile) -> Vec<String> {
    let mut runtimes = profile
        .game_id
        .as_deref()
        .and_then(game_definition_by_id)
        .map(|definition| definition.bootstrap_runtimes.clone())
        .unwrap_or_default();

    if runtime_definition_by_id(&profile.loader).is_some() {
        push_unique_string_value(&mut runtimes, &profile.loader);
    }
    runtimes
}

fn runtime_id_for_dependency(profile: &GameProfile, dependency: &DependencySpec) -> Option<String> {
    if let Some(runtime) = runtime_from_dependency(dependency) {
        return Some(runtime.to_string());
    }

    profile_runtime_ids(profile).into_iter().find(|runtime| {
        dependency_key(&known_runtime_dependency(profile, runtime.as_str()))
            == dependency_key(dependency)
    })
}

fn parse_thunderstore_dependency(dependency: &str) -> DependencySpec {
    let mut parts = dependency.split('-').collect::<Vec<_>>();
    let version = parts.pop().map(|value| value.to_string());
    let package_name = parts.pop().unwrap_or(dependency);
    let team_name = parts.join("-");
    let name = if team_name.is_empty() {
        dependency.to_string()
    } else {
        format!("{}/{}", team_name, package_name)
    };

    DependencySpec {
        id: format!("thunderstore:{}", dependency),
        name,
        version,
        provider: "thunderstore".to_string(),
        required: true,
        status: "missing".to_string(),
        source: Some("manifest.json".to_string()),
        notes: None,
    }
}

fn install_dependencies_for_plan(
    store_root: &Path,
    profile: &GameProfile,
    plan: &InstallPlan,
    visited_dependencies: &mut HashSet<String>,
    dependency_depth: usize,
) -> Result<Vec<String>, String> {
    let mut warnings = Vec::new();

    for dependency in &plan.dependencies {
        if !dependency.required {
            continue;
        }
        let dependency = refresh_dependency_status(store_root, profile, dependency);
        if dependency.status == "already-installed" {
            continue;
        }

        match install_dependency_by_provider(
            store_root,
            profile,
            &dependency,
            visited_dependencies,
            dependency_depth + 1,
        ) {
            Ok(mut dependency_warnings) => warnings.append(&mut dependency_warnings),
            Err(error) if dependency.required => return Err(error),
            Err(error) => warnings.push(error),
        }
    }

    Ok(warnings)
}

fn install_dependency_by_provider(
    store_root: &Path,
    profile: &GameProfile,
    dependency: &DependencySpec,
    visited_dependencies: &mut HashSet<String>,
    dependency_depth: usize,
) -> Result<Vec<String>, String> {
    match dependency.provider.as_str() {
        "thunderstore" => install_thunderstore_dependency(
            store_root,
            profile,
            dependency,
            visited_dependencies,
            dependency_depth,
        ),
        "github-release" => resolve_github_release_dependency(dependency).and_then(|release| {
            install_release_dependency(
                store_root,
                profile,
                dependency,
                release,
                visited_dependencies,
                dependency_depth,
            )
        }),
        "bepinbuilds" => resolve_bepinbuilds_dependency(dependency).and_then(|release| {
            install_release_dependency(
                store_root,
                profile,
                dependency,
                release,
                visited_dependencies,
                dependency_depth,
            )
        }),
        "nexus" | "curseforge" | "overwolf" | "modio" => {
            Err(platform_provider_pending_message(dependency))
        }
        "manual" | "known-runtime" => Err(format!(
            "{} needs a manual or game-specific provider rule before UniLoader can install it automatically.",
            dependency.name
        )),
        _ => Err(format!(
            "{} uses unsupported dependency provider '{}'.",
            dependency.name, dependency.provider
        )),
    }
}

fn platform_provider_pending_message(dependency: &DependencySpec) -> String {
    let platform = match dependency.provider.as_str() {
        "nexus" => "Nexus Mods",
        "curseforge" | "overwolf" => "CurseForge/Overwolf",
        "modio" => "Mod.io",
        _ => dependency.provider.as_str(),
    };

    format!(
        "{} is hosted on {}, which needs an official API/auth integration before UniLoader can download it automatically.",
        dependency.name, platform
    )
}

fn discover_online_mods_for_profile(
    store_root: &Path,
    profile: &GameProfile,
    settings: &AppSettings,
    page: usize,
    page_size: usize,
    sort: &str,
    query: &str,
) -> Result<DiscoveryPage, String> {
    let page = page.max(1);
    let page_size = page_size.clamp(1, MAX_DISCOVERY_PAGE_SIZE);
    let offset = (page - 1).saturating_mul(page_size);
    let needed = offset.saturating_add(page_size);
    let mut results = Vec::new();
    let mut provider_errors = Vec::new();
    let mut provider_total = 0_usize;

    if let Some(community) = thunderstore_community_for_profile(profile) {
        match discover_thunderstore_community_mods(store_root, profile, &community) {
            Ok(records) => {
                provider_total = provider_total.saturating_add(records.len());
                results.extend(records);
            }
            Err(error) => provider_errors.push(format!("Thunderstore: {error}")),
        }
    }

    if let Some(domain) = nexus_domain_for_profile(profile) {
        let fetch_count = if query.trim().is_empty() {
            needed
        } else {
            needed.saturating_mul(5).clamp(200, 500)
        };
        match discover_nexus_mods_for_profile(
            profile,
            &domain,
            settings.nexus_api_ready(),
            fetch_count,
            sort,
        ) {
            Ok((records, total)) => {
                provider_total = provider_total.saturating_add(total);
                results.extend(records);
            }
            Err(error) => provider_errors.push(format!("Nexus Mods: {error}")),
        }
    }

    let unfiltered_result_count = results.len();
    results.retain(|record| !is_external_mod_manager_listing(record));
    provider_total = provider_total.saturating_sub(unfiltered_result_count - results.len());

    let normalized_query = query.trim().to_lowercase();
    if !normalized_query.is_empty() {
        results.retain(|record| online_mod_matches_query(record, &normalized_query));
        provider_total = results.len();
    }
    sort_online_mods(&mut results, sort);
    if results.is_empty() && !provider_errors.is_empty() {
        return Err(provider_errors.join(" "));
    }

    if page > 1 && !provider_errors.is_empty() {
        return Err(provider_errors.join(" "));
    }

    let items = results
        .into_iter()
        .skip(offset)
        .take(page_size)
        .collect::<Vec<_>>();
    Ok(DiscoveryPage {
        has_more: offset.saturating_add(items.len()) < provider_total,
        items,
        total: provider_total,
        page,
        page_size,
    })
}

fn is_external_mod_manager_listing(record: &OnlineModRecord) -> bool {
    let name = record.name.to_lowercase();
    let description = record.description.to_lowercase();
    let combined = format!("{name} {description}");

    let external_name_markers = [
        "gale mod manager",
        "nexus mod manager",
        "mod organizer 2",
        "r2modman",
        "thunderstore mod manager",
        "thunderstore app",
        "vortex mod manager",
        "vortex extension",
        "vortex support",
    ];
    if external_name_markers
        .iter()
        .any(|marker| name.contains(marker))
    {
        return true;
    }

    let in_game_markers = [
        "in-game mod manager",
        "in game mod manager",
        "ingame mod manager",
        "configuration manager",
        "config manager",
        "configuration menu",
        "config menu",
        "mod browser",
        "mod menu",
        "mod viewer",
    ];
    if in_game_markers
        .iter()
        .any(|marker| combined.contains(marker))
    {
        return false;
    }

    let external_listing_markers = [
        "thunderstore mod manager",
        "thunderstore manager",
        "thunderstore app",
        "r2modman",
        "r2modmanplus",
        "nexus mod manager",
        "mod organizer 2",
        "vortex mod manager",
        "vortex extension",
        "vortex support",
        "support for vortex",
        "support for thunderstore",
        "overwolf app",
        "curseforge app",
        "desktop mod manager",
        "external mod manager",
        "mod manager for thunderstore",
        "manager for nexus mods",
    ];
    if external_listing_markers
        .iter()
        .any(|marker| combined.contains(marker))
    {
        return true;
    }

    let describes_manager = combined.contains("mod manager")
        || combined.contains("mod-manager")
        || combined.contains("modmanager");
    let external_ecosystem = combined.contains("thunderstore")
        || combined.contains("nexus mods")
        || combined.contains("r2modman")
        || combined.contains("vortex")
        || combined.contains("overwolf")
        || combined.contains("curseforge");

    describes_manager && external_ecosystem
}

fn online_mod_matches_query(record: &OnlineModRecord, query: &str) -> bool {
    record.name.to_lowercase().contains(query)
}

fn sort_online_mods(records: &mut [OnlineModRecord], sort: &str) {
    records.sort_by(|first, second| match sort {
        "newest" => online_mod_timestamp(second)
            .cmp(&online_mod_timestamp(first))
            .then_with(|| second.downloads.cmp(&first.downloads)),
        "oldest" => online_mod_timestamp(first)
            .cmp(&online_mod_timestamp(second))
            .then_with(|| second.downloads.cmp(&first.downloads)),
        _ => second
            .downloads
            .cmp(&first.downloads)
            .then_with(|| second.rating_score.cmp(&first.rating_score))
            .then_with(|| first.name.to_lowercase().cmp(&second.name.to_lowercase())),
    });
}

fn online_mod_timestamp(record: &OnlineModRecord) -> i64 {
    record
        .updated_at
        .as_deref()
        .or(record.created_at.as_deref())
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.timestamp())
        .unwrap_or(0)
}

fn thunderstore_community_for_profile(profile: &GameProfile) -> Option<String> {
    if let Some(community) = profile
        .game_id
        .as_deref()
        .and_then(game_definition_by_id)
        .and_then(|definition| definition.thunderstore_community.clone())
    {
        return Some(community);
    }

    auto_detect_thunderstore_community(profile)
}

fn nexus_domain_for_profile(profile: &GameProfile) -> Option<String> {
    if let Some(domain) = profile
        .game_id
        .as_deref()
        .and_then(game_definition_by_id)
        .and_then(|definition| definition.nexus_game_domain.clone())
    {
        return Some(domain);
    }

    auto_detect_nexus_domain(profile)
}

fn verified_discovery_provider_game(
    profile: &GameProfile,
    provider: &str,
    supplied_provider_game_id: Option<&str>,
    package_provider_game_id: Option<&str>,
) -> Result<String, String> {
    let expected = match provider {
        "thunderstore" => thunderstore_community_for_profile(profile),
        "nexus" => nexus_domain_for_profile(profile),
        _ => None,
    }
    .ok_or_else(|| {
        format!(
            "UniLoader could not verify the {} game catalogue for {}.",
            provider_label(provider),
            profile_game_label(profile)
        )
    })?;
    let actual = package_provider_game_id
        .or(supplied_provider_game_id)
        .unwrap_or(expected.as_str());

    if let Some(supplied) = supplied_provider_game_id {
        if compact_provider_slug(supplied) != compact_provider_slug(actual) {
            return Err(format!(
                "The selected {} result no longer matches its provider game catalogue. Refresh Discovery and try again.",
                provider_label(provider)
            ));
        }
    }
    if compact_provider_slug(actual) != compact_provider_slug(&expected) {
        return Err(format!(
            "This {} mod belongs to '{}', but the selected profile is linked to '{}'. No files were installed.",
            provider_label(provider),
            actual,
            expected
        ));
    }

    Ok(expected)
}

fn auto_detect_thunderstore_community(profile: &GameProfile) -> Option<String> {
    cached_provider_mapping("thunderstore", profile, || {
        let client = provider_client().ok()?;
        let candidates = provider_slug_candidates(profile);
        parallel_first_map(&candidates, &|candidate| {
            thunderstore_community_available(&client, candidate).then(|| candidate.to_string())
        })
    })
}

fn thunderstore_community_available(client: &Client, community: &str) -> bool {
    let url = format!(
        "{}/{}/api/v1/package/",
        THUNDERSTORE_COMMUNITY_API_BASE,
        sanitize_url_path_segment(community)
    );
    client
        .get(url)
        .send()
        .map(|response| response.status().is_success())
        .unwrap_or(false)
}

fn auto_detect_nexus_domain(profile: &GameProfile) -> Option<String> {
    cached_provider_mapping("nexus", profile, || {
        let client = provider_client().ok()?;
        let slugs = provider_slug_candidates(profile);
        if let Some(domain) = parallel_first_map(&slugs, &|candidate| {
            nexus_domain_has_mods(&client, candidate).then(|| candidate.to_string())
        }) {
            return Some(domain);
        }

        let names = provider_name_candidates(profile);
        parallel_first_map(&names, &|name| {
            let domain = fetch_nexus_game_domain_by_name(&client, name)?;
            nexus_domain_has_mods(&client, &domain).then_some(domain)
        })
    })
}

fn cached_provider_mapping<F>(provider: &str, profile: &GameProfile, resolve: F) -> Option<String>
where
    F: FnOnce() -> Option<String>,
{
    let game_identity = Path::new(&profile.game_path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&profile.game_path))
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase();
    let cache_key = format!("{provider}:{game_identity}");
    if let Ok(cache) = PROVIDER_MAPPING_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        if let Some(entry) = cache.get(&cache_key) {
            let ttl = if entry.value.is_some() {
                Duration::from_secs(PROVIDER_MAPPING_CACHE_HOURS * 60 * 60)
            } else {
                Duration::from_secs(PROVIDER_MAPPING_NEGATIVE_CACHE_MINUTES * 60)
            };
            if entry.fetched_at.elapsed() < ttl {
                return entry.value.clone();
            }
        }
    }

    let value = resolve();
    if let Ok(mut cache) = PROVIDER_MAPPING_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        cache.insert(
            cache_key,
            ProviderMappingCacheEntry {
                fetched_at: Instant::now(),
                value: value.clone(),
            },
        );
    }
    value
}

fn parallel_first_map<T, F>(items: &[String], mapper: &F) -> Option<T>
where
    T: Send,
    F: Fn(&str) -> Option<T> + Sync,
{
    if items.is_empty() {
        return None;
    }
    let worker_count = items.len().min(4);
    std::thread::scope(|scope| {
        let handles = (0..worker_count)
            .map(|worker_index| {
                scope.spawn(move || {
                    items
                        .iter()
                        .enumerate()
                        .filter(|(index, _)| index % worker_count == worker_index)
                        .find_map(|(index, item)| mapper(item).map(|value| (index, value)))
                })
            })
            .collect::<Vec<_>>();

        handles
            .into_iter()
            .filter_map(|handle| handle.join().ok().flatten())
            .min_by_key(|(index, _)| *index)
            .map(|(_, value)| value)
    })
}

fn nexus_domain_has_mods(client: &Client, domain: &str) -> bool {
    fetch_nexus_mod_page(client, domain, 0, 1)
        .map(|page| page.total_count > 0 || !page.nodes.is_empty())
        .unwrap_or(false)
}

fn provider_name_candidates(profile: &GameProfile) -> Vec<String> {
    let mut names = Vec::new();
    let mut seen = HashSet::new();

    for text in provider_candidate_texts(profile) {
        push_unique_string(&mut names, &mut seen, text.clone());
        let readable = readable_provider_text(&text);
        push_unique_string(&mut names, &mut seen, readable);
    }

    for slug in provider_slug_candidates(profile) {
        for alias in provider_name_aliases(&slug) {
            push_unique_string(&mut names, &mut seen, alias);
        }
    }

    names.truncate(MAX_PROVIDER_CANDIDATES);
    names
}

fn provider_slug_candidates(profile: &GameProfile) -> Vec<String> {
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();

    for text in provider_candidate_texts(profile) {
        let readable = readable_provider_text(&text);
        for source in [text.as_str(), readable.as_str()] {
            let compact = compact_provider_slug(source);
            let hyphenated = hyphenated_provider_slug(source);
            push_provider_slug(&mut candidates, &mut seen, compact.clone());
            push_provider_slug(&mut candidates, &mut seen, hyphenated);

            for alias in provider_slug_aliases(&compact) {
                push_provider_slug(&mut candidates, &mut seen, alias);
            }
        }
    }

    candidates.truncate(MAX_PROVIDER_CANDIDATES);
    candidates
}

fn provider_candidate_texts(profile: &GameProfile) -> Vec<String> {
    let mut texts = Vec::new();
    let mut seen = HashSet::new();

    if let Some(folder_name) = Path::new(&profile.game_path)
        .file_name()
        .and_then(|name| name.to_str())
    {
        push_unique_string(&mut texts, &mut seen, folder_name.to_string());
    }

    let normalized_path = normalize_archive_path(&profile.game_path);
    push_unique_string(&mut texts, &mut seen, basename(&normalized_path));

    if let Some(game_id) = &profile.game_id {
        push_unique_string(&mut texts, &mut seen, game_id.clone());
        if let Some(definition) = game_definition_by_id(game_id) {
            push_unique_string(&mut texts, &mut seen, definition.display_name.clone());
        }
    }

    texts
}

fn infer_profile_foundation_runtime(profile: &GameProfile) -> Option<ProviderRuntimeInference> {
    if !profile_bootstrap_dependencies(profile).is_empty() {
        return None;
    }

    let thunderstore_community = thunderstore_community_for_profile(profile);
    let nexus_domain = nexus_domain_for_profile(profile);
    if thunderstore_community.is_none() && nexus_domain.is_none() {
        return None;
    }

    let cache_key = format!(
        "{}|{}|{}|{}",
        profile.engine.to_lowercase(),
        profile.loader.to_lowercase(),
        thunderstore_community.as_deref().unwrap_or_default(),
        nexus_domain.as_deref().unwrap_or_default()
    );
    let cache = PROFILE_RUNTIME_INFERENCE_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(cache) = cache.lock() {
        if let Some(entry) = cache.get(&cache_key) {
            let ttl = if entry.value.is_some() {
                Duration::from_secs(PROVIDER_MAPPING_CACHE_HOURS * 60 * 60)
            } else {
                Duration::from_secs(PROVIDER_MAPPING_NEGATIVE_CACHE_MINUTES * 60)
            };
            if entry.fetched_at.elapsed() < ttl {
                return entry.value.clone();
            }
        }
    }

    let mut supporters = HashMap::<String, HashSet<String>>::new();
    let mut providers = HashMap::<String, HashSet<String>>::new();
    let mut sampled_mods = 0usize;

    if let Some(community) = thunderstore_community.as_deref() {
        if let Ok(packages) = fetch_thunderstore_community_packages(community) {
            sampled_mods = sampled_mods.saturating_add(collect_thunderstore_runtime_votes(
                profile,
                community,
                &packages,
                &mut supporters,
                &mut providers,
            ));
        }
    }

    if let Some(domain) = nexus_domain.as_deref() {
        if let Ok(client) = provider_client() {
            if let Ok(page) = fetch_nexus_mod_page(&client, domain, 0, PROFILE_RUNTIME_SAMPLE_SIZE)
            {
                sampled_mods = sampled_mods.saturating_add(collect_nexus_runtime_votes(
                    profile,
                    domain,
                    &page.nodes,
                    &mut supporters,
                    &mut providers,
                ));
            }
        }
    }

    let value = choose_runtime_inference(supporters, providers, sampled_mods);
    if let Ok(mut cache) = cache.lock() {
        cache.insert(
            cache_key,
            RuntimeInferenceCacheEntry {
                fetched_at: Instant::now(),
                value: value.clone(),
            },
        );
    }
    value
}

fn collect_thunderstore_runtime_votes(
    profile: &GameProfile,
    community: &str,
    packages: &[ThunderstoreCommunityPackage],
    supporters: &mut HashMap<String, HashSet<String>>,
    providers: &mut HashMap<String, HashSet<String>>,
) -> usize {
    let package_index = packages
        .iter()
        .map(|package| {
            (
                format!(
                    "{}/{}",
                    package.owner.to_lowercase(),
                    package.name.to_lowercase()
                ),
                package,
            )
        })
        .collect::<HashMap<_, _>>();
    let dependency_package_keys = packages
        .iter()
        .filter_map(latest_active_thunderstore_version)
        .flat_map(|version| version.dependencies.iter())
        .filter_map(|dependency| parse_thunderstore_token(dependency, None))
        .map(|package_ref| {
            format!(
                "{}/{}",
                package_ref.namespace.to_lowercase(),
                package_ref.name.to_lowercase()
            )
        })
        .collect::<HashSet<_>>();
    let mut sampled = packages
        .iter()
        .filter(|package| {
            is_discoverable_thunderstore_package(package)
                && !dependency_package_keys.contains(&format!(
                    "{}/{}",
                    package.owner.to_lowercase(),
                    package.name.to_lowercase()
                ))
        })
        .collect::<Vec<_>>();
    sampled.sort_by_key(|package| {
        std::cmp::Reverse(
            latest_active_thunderstore_version(package)
                .map(|version| version.downloads)
                .unwrap_or_default(),
        )
    });
    sampled.truncate(PROFILE_RUNTIME_SAMPLE_SIZE);

    for package in &sampled {
        let root_key = format!(
            "thunderstore:{community}:{}/{}",
            package.owner.to_lowercase(),
            package.name.to_lowercase()
        );
        let Some(version) = latest_active_thunderstore_version(package) else {
            continue;
        };
        let mut queue = version
            .dependencies
            .iter()
            .cloned()
            .map(|dependency| (dependency, 1usize))
            .collect::<VecDeque<_>>();
        let mut visited = HashSet::new();
        let mut root_runtimes = HashSet::new();

        while let Some((dependency, depth)) = queue.pop_front() {
            if depth > PROFILE_RUNTIME_MAX_DEPENDENCY_DEPTH {
                continue;
            }
            let Some(package_ref) = parse_thunderstore_token(&dependency, None) else {
                continue;
            };
            let dependency_key = format!(
                "{}/{}",
                package_ref.namespace.to_lowercase(),
                package_ref.name.to_lowercase()
            );
            if !visited.insert(dependency_key.clone()) {
                continue;
            }

            if let Some(runtime_id) = runtime_id_for_provider_package(
                profile,
                "thunderstore",
                Some(&package_ref.namespace),
                &package_ref.name,
            ) {
                root_runtimes.insert(runtime_id);
            }

            if let Some(dependency_package) = package_index.get(&dependency_key) {
                if let Some(dependency_version) =
                    latest_active_thunderstore_version(dependency_package)
                {
                    queue.extend(
                        dependency_version
                            .dependencies
                            .iter()
                            .cloned()
                            .map(|nested| (nested, depth + 1)),
                    );
                }
            }
        }

        for runtime_id in root_runtimes {
            supporters
                .entry(runtime_id.clone())
                .or_default()
                .insert(root_key.clone());
            providers
                .entry(runtime_id)
                .or_default()
                .insert("Thunderstore".to_string());
        }
    }

    sampled.len()
}

fn collect_nexus_runtime_votes(
    profile: &GameProfile,
    domain: &str,
    nodes: &[NexusModNode],
    supporters: &mut HashMap<String, HashSet<String>>,
    providers: &mut HashMap<String, HashSet<String>>,
) -> usize {
    for node in nodes {
        let Some(mod_id) = node.mod_id else {
            continue;
        };
        let root_key = format!("nexus:{domain}/{mod_id}");
        let mut root_runtimes = HashSet::new();
        for requirement in node
            .mod_requirements
            .as_ref()
            .map(|requirements| requirements.nexus_requirements.nodes.as_slice())
            .unwrap_or_default()
        {
            if let Some(runtime_id) = runtime_id_for_nexus_requirement(profile, requirement) {
                root_runtimes.insert(runtime_id);
            }
        }
        for runtime_id in root_runtimes {
            supporters
                .entry(runtime_id.clone())
                .or_default()
                .insert(root_key.clone());
            providers
                .entry(runtime_id)
                .or_default()
                .insert("Nexus Mods".to_string());
        }
    }
    nodes.len()
}

fn choose_runtime_inference(
    supporters: HashMap<String, HashSet<String>>,
    providers: HashMap<String, HashSet<String>>,
    sampled_mods: usize,
) -> Option<ProviderRuntimeInference> {
    let mut ranked = supporters
        .into_iter()
        .map(|(runtime_id, mods)| (runtime_id, mods.len()))
        .collect::<Vec<_>>();
    ranked.sort_by(|first, second| second.1.cmp(&first.1).then_with(|| first.0.cmp(&second.0)));
    let (runtime_id, supporting_mods) = ranked.first()?.clone();
    if supporting_mods < PROFILE_RUNTIME_MIN_SUPPORT
        || ranked
            .get(1)
            .is_some_and(|second| second.1 >= supporting_mods)
    {
        return None;
    }

    let mut provider_names = providers
        .get(&runtime_id)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .collect::<Vec<_>>();
    provider_names.sort();
    Some(ProviderRuntimeInference {
        runtime_id,
        providers: provider_names,
        supporting_mods,
        sampled_mods,
    })
}

fn ensure_profile_route_knowledge(
    root: &Path,
    profile: &GameProfile,
    force_refresh: bool,
) -> RouteKnowledgeOutcome {
    let existing = read_profile_route_knowledge(root, &profile.id)
        .ok()
        .flatten();
    let mut knowledge = if !force_refresh
        && existing
            .as_ref()
            .is_some_and(profile_route_knowledge_is_fresh)
    {
        existing.unwrap()
    } else {
        learn_profile_route_knowledge(root, profile, existing.as_ref())
    };

    let outcome = apply_profile_route_knowledge(profile, &mut knowledge);
    if let Err(error) = write_profile_route_knowledge(root, &knowledge) {
        let mut outcome = outcome;
        outcome
            .warnings
            .push(format!("Could not save learned install routes: {error}"));
        return outcome;
    }
    outcome
}

fn profile_route_knowledge_is_fresh(knowledge: &ProfileRouteKnowledge) -> bool {
    if knowledge.version != PROFILE_ROUTE_KNOWLEDGE_VERSION {
        return false;
    }
    let Ok(learned_at) = chrono::DateTime::parse_from_rfc3339(&knowledge.learned_at) else {
        return false;
    };
    let age = Utc::now().signed_duration_since(learned_at.with_timezone(&Utc));
    let maximum_age = if knowledge.sampled_mods == 0 {
        chrono::Duration::minutes(PROFILE_ROUTE_NEGATIVE_CACHE_MINUTES)
    } else {
        chrono::Duration::hours(PROFILE_ROUTE_CACHE_HOURS)
    };
    age >= chrono::Duration::zero() && age <= maximum_age
}

fn learn_profile_route_knowledge(
    root: &Path,
    profile: &GameProfile,
    previous: Option<&ProfileRouteKnowledge>,
) -> ProfileRouteKnowledge {
    let settings = read_app_settings(root).ok();
    let nexus_api_key = settings
        .as_ref()
        .and_then(AppSettings::nexus_api_key)
        .map(str::to_string);
    let (documents, warnings) = collect_profile_route_documents(profile, nexus_api_key.as_deref());
    let mut knowledge = build_profile_route_knowledge(profile, &documents, warnings);

    if let Some(previous) = previous {
        for retained in previous
            .routes
            .iter()
            .filter(|route| route.package_verified)
        {
            merge_learned_route(&mut knowledge.routes, retained.clone());
        }
    }

    knowledge.routes.sort_by(|first, second| {
        first
            .relative_path
            .to_lowercase()
            .cmp(&second.relative_path.to_lowercase())
    });
    knowledge.routes.truncate(32);
    knowledge
}

fn collect_profile_route_documents(
    profile: &GameProfile,
    nexus_api_key: Option<&str>,
) -> (Vec<ProviderRouteDocument>, Vec<String>) {
    let mut documents = Vec::new();
    let mut warnings = Vec::new();

    if let Some(community) = thunderstore_community_for_profile(profile) {
        match fetch_thunderstore_route_documents(&community) {
            Ok(mut provider_documents) => documents.append(&mut provider_documents),
            Err(error) => warnings.push(format!(
                "Thunderstore route metadata was unavailable: {error}"
            )),
        }
    }

    if let Some(domain) = nexus_domain_for_profile(profile) {
        match fetch_nexus_route_documents(&domain, nexus_api_key) {
            Ok(mut provider_documents) => documents.append(&mut provider_documents),
            Err(error) => warnings.push(format!("Nexus route metadata was unavailable: {error}")),
        }
    }

    (documents, warnings)
}

fn fetch_thunderstore_route_documents(
    community: &str,
) -> Result<Vec<ProviderRouteDocument>, String> {
    let packages = fetch_thunderstore_community_packages(community)?;
    let mut sampled = packages
        .into_iter()
        .filter(is_discoverable_thunderstore_package)
        .filter_map(|package| {
            let version = latest_active_thunderstore_version(&package)?.clone();
            Some((package, version))
        })
        .collect::<Vec<_>>();
    sampled.sort_by_key(|(_, version)| std::cmp::Reverse(version.downloads));
    sampled.truncate(PROFILE_ROUTE_SAMPLE_SIZE);

    let client = thunderstore_client()?;
    let mut documents = Vec::new();
    for (batch_index, chunk) in sampled.chunks(PROFILE_ROUTE_FETCH_CONCURRENCY).enumerate() {
        let include_readme =
            batch_index * PROFILE_ROUTE_FETCH_CONCURRENCY < PROFILE_ROUTE_FULL_TEXT_SAMPLE_SIZE;
        let batch_documents = std::thread::scope(|scope| {
            let handles = chunk
                .iter()
                .cloned()
                .enumerate()
                .map(|(index, (package, version))| {
                    let client = client.clone();
                    let fetch_readme = include_readme
                        && batch_index * PROFILE_ROUTE_FETCH_CONCURRENCY + index
                            < PROFILE_ROUTE_FULL_TEXT_SAMPLE_SIZE;
                    let community = community.to_string();
                    scope.spawn(move || {
                        let package_ref = ThunderstorePackageRef {
                            namespace: package.owner.clone(),
                            name: package.name.clone(),
                            version: Some(version.version_number.clone()),
                        };
                        let readme = fetch_readme
                            .then(|| {
                                fetch_thunderstore_package_readme_with_client(
                                    &client,
                                    &package_ref,
                                    &version.version_number,
                                )
                                .ok()
                            })
                            .flatten()
                            .unwrap_or_default();
                        let text = [version.description.as_str(), readme.as_str()]
                            .into_iter()
                            .filter(|value| !value.trim().is_empty())
                            .collect::<Vec<_>>()
                            .join("\n");
                        ProviderRouteDocument {
                            provider: "Thunderstore".to_string(),
                            mod_id: format!(
                                "thunderstore:{community}:{}/{}",
                                package.owner, package.name
                            ),
                            mod_name: humanize_mod_display_name(&package.full_name),
                            text,
                        }
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .filter_map(|handle| handle.join().ok())
                .collect::<Vec<_>>()
        });
        documents.extend(batch_documents);
    }
    Ok(documents)
}

fn fetch_thunderstore_package_readme_with_client(
    client: &Client,
    package_ref: &ThunderstorePackageRef,
    version: &str,
) -> Result<String, String> {
    let url = format!(
        "{}/{}/{}/{}/readme/",
        THUNDERSTORE_API_BASE,
        sanitize_url_path_segment(&package_ref.namespace),
        sanitize_url_path_segment(&package_ref.name),
        sanitize_url_path_segment(version)
    );
    client
        .get(url)
        .send()
        .map_err(error_to_string)?
        .error_for_status()
        .map_err(error_to_string)?
        .json::<ThunderstoreMarkdownResponse>()
        .map(|response| response.markdown)
        .map_err(error_to_string)
}

fn thunderstore_route_document(
    package_ref: &ThunderstorePackageRef,
    version: &ThunderstoreVersion,
) -> ProviderRouteDocument {
    let readme = thunderstore_client()
        .and_then(|client| {
            fetch_thunderstore_package_readme_with_client(
                &client,
                package_ref,
                &version.version_number,
            )
        })
        .unwrap_or_default();
    ProviderRouteDocument {
        provider: "Thunderstore".to_string(),
        mod_id: thunderstore_package_id(package_ref),
        mod_name: humanize_mod_display_name(&version.full_name),
        text: [version.description.as_str(), readme.as_str()]
            .into_iter()
            .filter(|value| !value.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn fetch_nexus_route_documents(
    domain: &str,
    api_key: Option<&str>,
) -> Result<Vec<ProviderRouteDocument>, String> {
    let client = provider_client()?;
    let page = fetch_nexus_mod_page(&client, domain, 0, PROFILE_ROUTE_SAMPLE_SIZE)?;
    let nodes = page
        .nodes
        .into_iter()
        .filter(|node| node.mod_id.is_some())
        .collect::<Vec<_>>();
    let mut details_by_id = HashMap::<u64, NexusModDetails>::new();

    if let Some(api_key) = api_key.filter(|value| !value.trim().is_empty()) {
        for chunk in nodes[..nodes.len().min(PROFILE_ROUTE_FULL_TEXT_SAMPLE_SIZE)]
            .chunks(PROFILE_ROUTE_FETCH_CONCURRENCY)
        {
            let batch_details = std::thread::scope(|scope| {
                let handles = chunk
                    .iter()
                    .filter_map(|node| node.mod_id)
                    .map(|mod_id| {
                        let client = client.clone();
                        let api_key = api_key.to_string();
                        let domain = domain.to_string();
                        scope.spawn(move || {
                            fetch_nexus_mod_details(&client, &api_key, &domain, mod_id)
                                .ok()
                                .map(|details| (mod_id, details))
                        })
                    })
                    .collect::<Vec<_>>();
                handles
                    .into_iter()
                    .filter_map(|handle| handle.join().ok().flatten())
                    .collect::<Vec<_>>()
            });
            details_by_id.extend(batch_details);
        }
    }

    Ok(nodes
        .into_iter()
        .filter_map(|node| {
            let mod_id = node.mod_id?;
            let details = details_by_id.remove(&mod_id).unwrap_or_default();
            let name = non_empty_string(details.name.trim())
                .or(node.name)
                .unwrap_or_else(|| format!("Nexus mod {mod_id}"));
            let summary = node.summary.unwrap_or_default();
            let text = [
                summary.as_str(),
                details.summary.as_str(),
                details.description.as_str(),
            ]
            .into_iter()
            .filter(|value| !value.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");
            Some(ProviderRouteDocument {
                provider: "Nexus Mods".to_string(),
                mod_id: format!("nexus:{domain}/{mod_id}"),
                mod_name: name,
                text,
            })
        })
        .collect())
}

fn fetch_nexus_mod_details(
    client: &Client,
    api_key: &str,
    domain: &str,
    mod_id: u64,
) -> Result<NexusModDetails, String> {
    let url = format!(
        "https://api.nexusmods.com/v1/games/{}/mods/{}.json",
        sanitize_url_path_segment(domain),
        mod_id
    );
    nexus_api_get(client, &url, api_key)
        .send()
        .map_err(error_to_string)?
        .error_for_status()
        .map_err(error_to_string)?
        .json::<NexusModDetails>()
        .map_err(error_to_string)
}

fn nexus_mod_details_icon_url(details: &NexusModDetails) -> Option<String> {
    [
        details.thumbnail_large_url.as_deref(),
        details.picture_url.as_deref(),
        details.thumbnail_url.as_deref(),
    ]
    .into_iter()
    .find_map(|value| value.and_then(|value| non_empty_string(value.trim())))
}

fn nexus_mod_node_icon_url(node: &NexusModNode) -> Option<String> {
    [
        node.thumbnail_large_url.as_deref(),
        node.picture_url.as_deref(),
        node.thumbnail_url.as_deref(),
    ]
    .into_iter()
    .find_map(|value| value.and_then(|value| non_empty_string(value.trim())))
}

fn backfill_installed_mod_artwork(
    store_root: &Path,
    profile: &GameProfile,
    settings: Option<&AppSettings>,
    store: &mut StoreFile<InstalledModRecord>,
) -> usize {
    if !store.items.iter().any(|record| {
        record.profile_id == profile.id
            && record.last_status != "removed"
            && record.runtime_id.is_none()
            && record.icon_url.is_none()
    }) {
        return 0;
    }

    let Ok(client) = provider_client() else {
        return 0;
    };
    let nexus_api_key = settings.and_then(AppSettings::nexus_api_key);
    let nexus_domain = nexus_domain_for_profile(profile);
    let mut changed = 0;

    for record in store.items.iter_mut().filter(|record| {
        record.profile_id == profile.id
            && record.last_status != "removed"
            && record.runtime_id.is_none()
            && record.icon_url.is_none()
    }) {
        let mut recovered_package_id = None;
        let icon_url = record
            .package_id
            .as_deref()
            .and_then(|package_id| installed_provider_artwork(&client, nexus_api_key, package_id))
            .or_else(|| {
                recover_legacy_nexus_artwork(
                    &client,
                    nexus_api_key,
                    nexus_domain.as_deref(),
                    record,
                )
                .map(|(package_id, icon_url)| {
                    recovered_package_id = Some(package_id);
                    icon_url
                })
            });

        let Some(icon_url) = icon_url else {
            continue;
        };
        record.icon_url = Some(icon_url);
        if record.package_id.is_none() {
            record.package_id = recovered_package_id;
        }
        let _ = write_receipt(store_root, profile, record);
        changed += 1;
    }

    changed
}

fn installed_provider_artwork(
    client: &Client,
    nexus_api_key: Option<&str>,
    package_id: &str,
) -> Option<String> {
    if package_id.starts_with("nexus:") {
        let api_key = nexus_api_key?;
        let (domain, mod_id) = parse_nexus_online_mod_id(package_id).ok()?;
        return fetch_nexus_mod_details(client, api_key, &domain, mod_id)
            .ok()
            .and_then(|details| nexus_mod_details_icon_url(&details));
    }

    let raw_id = package_id.strip_prefix("thunderstore:")?;
    let package_ref = parse_thunderstore_token(raw_id, None)?;
    fetch_thunderstore_package_version(&package_ref)
        .ok()
        .and_then(|version| version.icon)
        .and_then(|icon| non_empty_string(icon.trim()))
}

fn recover_legacy_nexus_artwork(
    client: &Client,
    nexus_api_key: Option<&str>,
    domain: Option<&str>,
    record: &InstalledModRecord,
) -> Option<(String, String)> {
    let domain = domain?;

    if let (Some(api_key), Some(mod_id)) = (
        nexus_api_key,
        nexus_mod_id_from_download_name(&record.archive_name),
    ) {
        if let Ok(details) = fetch_nexus_mod_details(client, api_key, domain, mod_id) {
            if let Some(icon_url) = nexus_mod_details_icon_url(&details) {
                return Some((format!("nexus:{domain}/{mod_id}"), icon_url));
            }
        }
    }

    let display_name = record
        .display_name
        .as_deref()
        .unwrap_or(&record.archive_name);
    let search_term = legacy_mod_artwork_search_term(display_name)?;
    let expected_key = legacy_mod_artwork_match_key(&search_term);
    if expected_key.len() < 5 {
        return None;
    }

    let matches = fetch_nexus_mods_by_name(client, domain, &search_term)
        .ok()?
        .into_iter()
        .filter(|node| {
            node.name
                .as_deref()
                .map(legacy_mod_artwork_match_key)
                .as_deref()
                == Some(expected_key.as_str())
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return None;
    }

    let node = matches.into_iter().next()?;
    let mod_id = node.mod_id?;
    let icon_url = nexus_mod_node_icon_url(&node)?;
    Some((format!("nexus:{domain}/{mod_id}"), icon_url))
}

fn fetch_nexus_mods_by_name(
    client: &Client,
    domain: &str,
    name: &str,
) -> Result<Vec<NexusModNode>, String> {
    const NEXUS_NAME_QUERY: &str = r#"
        query UniLoaderFindModArtwork(
          $filter: ModsFilter,
          $offset: Int,
          $count: Int
        ) {
          mods(filter: $filter, offset: $offset, count: $count) {
            totalCount
            nodes {
              modId
              name
              pictureUrl
              thumbnailUrl
              thumbnailLargeUrl
            }
          }
        }
    "#;
    let body = serde_json::json!({
        "query": NEXUS_NAME_QUERY,
        "variables": {
            "filter": {
                "op": "AND",
                "gameDomainName": [{ "value": domain, "op": "EQUALS" }],
                "name": [{ "value": name, "op": "WILDCARD" }],
                "adultContent": [{ "value": false, "op": "EQUALS" }],
                "status": [{ "value": "published", "op": "EQUALS" }]
            },
            "offset": 0,
            "count": 20
        }
    });
    let response = client
        .post(NEXUS_GRAPHQL_API_BASE)
        .json(&body)
        .send()
        .map_err(error_to_string)?
        .error_for_status()
        .map_err(error_to_string)?
        .json::<NexusGraphqlResponse>()
        .map_err(error_to_string)?;

    if !response.errors.is_empty() {
        return Err(response
            .errors
            .into_iter()
            .map(|error| error.message)
            .collect::<Vec<_>>()
            .join("; "));
    }
    response
        .data
        .map(|data| data.mods.nodes)
        .ok_or_else(|| "Nexus Mods returned no artwork data.".to_string())
}

fn nexus_mod_id_from_download_name(archive_name: &str) -> Option<u64> {
    let file_name = basename(&normalize_archive_path(archive_name));
    let lower = file_name.to_ascii_lowercase();
    let stem = [".zip", ".7z", ".rar"]
        .iter()
        .find_map(|extension| {
            lower
                .ends_with(extension)
                .then(|| file_name[..file_name.len().saturating_sub(extension.len())].to_string())
        })
        .unwrap_or(file_name);
    let segments = stem.split('-').collect::<Vec<_>>();

    segments.windows(3).find_map(|window| {
        let mod_id = window[0].parse::<u64>().ok()?;
        window[1].parse::<u64>().ok()?;
        let timestamp = window[2];
        (timestamp.len() >= 9
            && timestamp
                .chars()
                .all(|character| character.is_ascii_digit()))
        .then_some(mod_id)
    })
}

fn legacy_mod_artwork_search_term(value: &str) -> Option<String> {
    let humanized = humanize_mod_display_name(value);
    let mut words = humanized
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    while words
        .last()
        .map(|word| is_mod_variant_suffix(word))
        .unwrap_or(false)
    {
        words.pop();
    }
    non_empty_string(words.join(" ").trim())
}

fn legacy_mod_artwork_match_key(value: &str) -> String {
    legacy_mod_artwork_search_term(value)
        .unwrap_or_default()
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(|character| character.to_lowercase())
        .collect()
}

fn is_mod_variant_suffix(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let without_prefix = lower.strip_prefix('x').unwrap_or(&lower);
    let without_suffix = without_prefix.strip_suffix('x').unwrap_or(without_prefix);
    without_suffix
        .chars()
        .all(|character| character.is_ascii_digit())
        || lower
            .strip_suffix('h')
            .map(|number| number.chars().all(|character| character.is_ascii_digit()))
            .unwrap_or(false)
}

fn build_profile_route_knowledge(
    profile: &GameProfile,
    documents: &[ProviderRouteDocument],
    warnings: Vec<String>,
) -> ProfileRouteKnowledge {
    #[derive(Default)]
    struct RouteAggregate {
        relative_path: String,
        adapter_id: String,
        scopes: HashSet<String>,
        supporters: HashSet<String>,
        providers: HashSet<String>,
        evidence: Vec<RouteEvidence>,
    }

    let mut aggregates = HashMap::<String, RouteAggregate>::new();
    for document in documents {
        let mut document_routes = HashSet::new();
        for candidate in extract_install_route_candidates(profile, &document.text) {
            let key = install_route_key(&candidate.adapter_id, &candidate.relative_path);
            if !document_routes.insert(key.clone()) {
                continue;
            }
            let aggregate = aggregates.entry(key).or_default();
            aggregate.relative_path = candidate.relative_path;
            aggregate.adapter_id = candidate.adapter_id;
            aggregate.scopes.extend(candidate.scopes);
            aggregate.supporters.insert(document.mod_id.clone());
            aggregate.providers.insert(document.provider.clone());
            if aggregate.evidence.len() < PROFILE_ROUTE_MAX_EVIDENCE {
                aggregate.evidence.push(RouteEvidence {
                    provider: document.provider.clone(),
                    mod_id: document.mod_id.clone(),
                    mod_name: document.mod_name.clone(),
                    excerpt: candidate.excerpt,
                });
            }
        }
    }

    let mut routes = aggregates
        .into_values()
        .filter_map(|aggregate| {
            if !validate_learned_route_shape(&aggregate.adapter_id, &aggregate.relative_path) {
                return None;
            }
            let supporting_mods = aggregate.supporters.len();
            let route_exists = safe_join(Path::new(&profile.game_path), &aggregate.relative_path)
                .map(|path| path.is_dir())
                .unwrap_or(false);
            let compatible = route_adapter_compatible(profile, &aggregate.adapter_id);
            let trusted = supporting_mods >= PROFILE_ROUTE_MIN_SUPPORT
                && (compatible || route_exists || profile.engine == "unknown");
            let confidence = (0.58
                + 0.1 * supporting_mods.min(4) as f64
                + if aggregate.providers.len() > 1 {
                    0.08
                } else {
                    0.0
                }
                + if route_exists { 0.05 } else { 0.0 })
            .min(0.98);
            let mut scopes = aggregate.scopes.into_iter().collect::<Vec<_>>();
            scopes.sort();
            let mut providers = aggregate.providers.into_iter().collect::<Vec<_>>();
            providers.sort();
            Some(LearnedInstallRoute {
                relative_path: aggregate.relative_path,
                adapter_id: aggregate.adapter_id,
                scopes,
                confidence,
                supporting_mods,
                providers,
                evidence: aggregate.evidence,
                trusted,
                package_verified: false,
                created: false,
            })
        })
        .collect::<Vec<_>>();
    routes.sort_by(|first, second| {
        second
            .trusted
            .cmp(&first.trusted)
            .then_with(|| {
                second
                    .confidence
                    .partial_cmp(&first.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| first.relative_path.cmp(&second.relative_path))
    });

    let mut providers = documents
        .iter()
        .map(|document| document.provider.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    providers.sort();
    let sampled_mods = documents
        .iter()
        .map(|document| document.mod_id.to_lowercase())
        .collect::<HashSet<_>>()
        .len();

    ProfileRouteKnowledge {
        version: PROFILE_ROUTE_KNOWLEDGE_VERSION,
        profile_id: profile.id.clone(),
        learned_at: now_string(),
        sampled_mods,
        providers,
        routes,
        warnings,
    }
}

fn merge_learned_route(routes: &mut Vec<LearnedInstallRoute>, incoming: LearnedInstallRoute) {
    let key = install_route_key(&incoming.adapter_id, &incoming.relative_path);
    if let Some(existing) = routes
        .iter_mut()
        .find(|route| install_route_key(&route.adapter_id, &route.relative_path) == key)
    {
        for scope in incoming.scopes {
            push_unique_string_value(&mut existing.scopes, &scope);
        }
        for provider in incoming.providers {
            push_unique_string_value(&mut existing.providers, &provider);
        }
        for evidence in incoming.evidence {
            if existing.evidence.len() >= PROFILE_ROUTE_MAX_EVIDENCE {
                break;
            }
            if !existing.evidence.iter().any(|item| {
                item.mod_id.eq_ignore_ascii_case(&evidence.mod_id)
                    && item.provider.eq_ignore_ascii_case(&evidence.provider)
            }) {
                existing.evidence.push(evidence);
            }
        }
        existing.supporting_mods = existing.supporting_mods.max(incoming.supporting_mods).max(
            existing
                .evidence
                .iter()
                .map(|evidence| evidence.mod_id.to_lowercase())
                .collect::<HashSet<_>>()
                .len(),
        );
        existing.confidence = existing.confidence.max(incoming.confidence);
        existing.trusted |= incoming.trusted;
        existing.package_verified |= incoming.package_verified;
        existing.created |= incoming.created;
    } else {
        routes.push(incoming);
    }
}

fn install_route_key(adapter_id: &str, relative_path: &str) -> String {
    format!(
        "{}|{}",
        adapter_id.to_lowercase(),
        normalize_archive_path(relative_path).to_lowercase()
    )
}

fn extract_install_route_candidates(
    profile: &GameProfile,
    text: &str,
) -> Vec<InstallRouteCandidate> {
    let scan_text = provider_text_for_route_scan(text);
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();

    for line in scan_text.lines() {
        let context = line.trim();
        if context.is_empty() {
            continue;
        }
        for token in context.split(|character: char| {
            character.is_whitespace()
                || matches!(
                    character,
                    '\'' | '"' | '`' | '=' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | '|'
                )
        }) {
            let Some((relative_path, adapter_id)) =
                normalize_install_route_candidate(profile, token)
            else {
                continue;
            };
            let key = install_route_key(&adapter_id, &relative_path);
            if !seen.insert(key) {
                continue;
            }
            candidates.push(InstallRouteCandidate {
                relative_path,
                adapter_id,
                scopes: install_route_scopes(context),
                excerpt: compact_route_excerpt(context),
            });
        }
    }

    candidates
}

fn provider_text_for_route_scan(text: &str) -> String {
    let decoded = decode_provider_html_entities(text);
    let characters = decoded.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(decoded.len());
    let mut index = 0usize;

    while index < characters.len() {
        if characters[index] != '<' {
            output.push(characters[index]);
            index += 1;
            continue;
        }
        let Some(relative_end) = characters[index + 1..]
            .iter()
            .position(|character| *character == '>')
        else {
            output.push('<');
            index += 1;
            continue;
        };
        let end = index + 1 + relative_end;
        let content = characters[index + 1..end].iter().collect::<String>();
        let tag_name = content
            .trim()
            .trim_start_matches('/')
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_end_matches('/')
            .to_lowercase();
        if is_provider_html_tag(&tag_name) {
            if matches!(
                tag_name.as_str(),
                "br" | "p" | "div" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "pre"
            ) {
                output.push('\n');
            } else {
                output.push(' ');
            }
        } else {
            output.push('<');
            for character in content.chars() {
                output.push(if character.is_whitespace() {
                    '_'
                } else {
                    character
                });
            }
            output.push('>');
        }
        index = end + 1;
    }
    output
}

fn decode_provider_html_entities(value: &str) -> String {
    let mut decoded = String::with_capacity(value.len());
    let mut cursor = 0usize;

    while let Some(relative_start) = value[cursor..].find('&') {
        let start = cursor + relative_start;
        decoded.push_str(&value[cursor..start]);
        let Some(relative_end) = value[start + 1..].find(';') else {
            decoded.push_str(&value[start..]);
            return decoded;
        };
        let end = start + 1 + relative_end;
        let entity = &value[start + 1..end];
        if entity.len() <= 16 {
            if let Some(character) = decode_provider_html_entity(entity) {
                decoded.push(character);
                cursor = end + 1;
                continue;
            }
        }

        decoded.push('&');
        cursor = start + 1;
    }

    decoded.push_str(&value[cursor..]);
    decoded
}

fn decode_provider_html_entity(entity: &str) -> Option<char> {
    match entity.to_ascii_lowercase().as_str() {
        "amp" => Some('&'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "nbsp" => Some(' '),
        numeric if numeric.starts_with("#x") => u32::from_str_radix(&numeric[2..], 16)
            .ok()
            .and_then(char::from_u32),
        numeric if numeric.starts_with('#') => {
            numeric[1..].parse::<u32>().ok().and_then(char::from_u32)
        }
        _ => None,
    }
}

fn is_provider_html_tag(tag_name: &str) -> bool {
    matches!(
        tag_name,
        "a" | "b"
            | "blockquote"
            | "br"
            | "code"
            | "div"
            | "em"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "i"
            | "img"
            | "li"
            | "ol"
            | "p"
            | "pre"
            | "span"
            | "strong"
            | "table"
            | "tbody"
            | "td"
            | "th"
            | "thead"
            | "tr"
            | "ul"
    )
}

fn normalize_install_route_candidate(
    profile: &GameProfile,
    raw_token: &str,
) -> Option<(String, String)> {
    let mut token = raw_token.trim_matches(|character: char| {
        !(character.is_ascii_alphanumeric()
            || matches!(
                character,
                '/' | '\\' | '.' | '_' | '-' | '~' | '<' | '>' | ':'
            ))
    });
    if token.is_empty()
        || token.to_lowercase().contains("://")
        || token.to_lowercase().starts_with("www.")
    {
        return None;
    }
    if let Some(colon_index) = token.rfind(':') {
        if colon_index == 1
            && token
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_alphabetic())
        {
            return None;
        }
        token = &token[colon_index + 1..];
    }

    let mut normalized = token.replace('\\', "/");
    while normalized.contains("//") {
        normalized = normalized.replace("//", "/");
    }
    normalized = normalized
        .trim_matches(|character: char| {
            !(character.is_ascii_alphanumeric()
                || matches!(character, '/' | '.' | '_' | '-' | '~' | '<' | '>'))
        })
        .trim_start_matches("./")
        .trim_start_matches('/')
        .trim_end_matches('.')
        .to_string();

    if normalized.starts_with('<') {
        let placeholder_end = normalized.find('>')?;
        let placeholder = &normalized[1..placeholder_end];
        if !profile_route_placeholder_matches(profile, placeholder) {
            return None;
        }
        normalized = normalized[placeholder_end + 1..]
            .trim_start_matches('/')
            .to_string();
    }

    let aliases = profile_route_root_aliases(profile);
    let mut parts = normalized
        .split('/')
        .filter(|part| !part.trim().is_empty())
        .map(|part| part.trim().to_string())
        .collect::<Vec<_>>();

    while parts
        .first()
        .is_some_and(|part| part.chars().all(|character| character == '.'))
    {
        parts.remove(0);
    }
    if let Some(alias_index) = parts
        .iter()
        .position(|part| aliases.contains(&compact_provider_slug(part)))
    {
        parts.drain(..=alias_index);
    }
    if parts.is_empty() || parts.len() > 12 {
        return None;
    }
    if parts.iter().any(|part| !safe_install_route_component(part)) {
        return None;
    }

    let (relative_path, adapter_id) = canonical_install_route(&parts)?;
    if relative_path.len() > 260
        || !validate_learned_route_shape(&adapter_id, &relative_path)
        || !route_has_safe_internal_root(profile, &relative_path, &adapter_id)
    {
        return None;
    }
    safe_join(Path::new(&profile.game_path), &relative_path).ok()?;
    Some((relative_path, adapter_id))
}

fn profile_route_placeholder_matches(profile: &GameProfile, placeholder: &str) -> bool {
    let compact = compact_provider_slug(placeholder);
    if matches!(
        compact.as_str(),
        "game" | "gamefolder" | "gamedirectory" | "gameroot" | "installfolder" | "steamlocation"
    ) {
        return true;
    }
    profile_route_root_aliases(profile).contains(&compact)
}

fn profile_route_root_aliases(profile: &GameProfile) -> HashSet<String> {
    let mut aliases = HashSet::new();
    aliases.insert(compact_provider_slug(&profile.name));
    if let Some(folder_name) = Path::new(&profile.game_path)
        .file_name()
        .and_then(|name| name.to_str())
    {
        aliases.insert(compact_provider_slug(folder_name));
    }
    if let Some(definition) = profile.game_id.as_deref().and_then(game_definition_by_id) {
        aliases.insert(compact_provider_slug(&definition.display_name));
        aliases.insert(compact_provider_slug(&definition.id));
    }
    aliases.retain(|alias| !alias.is_empty());
    aliases
}

fn safe_install_route_component(component: &str) -> bool {
    let trimmed = component.trim();
    let windows_trimmed = trimmed.trim_end_matches(['.', ' ']);
    if trimmed.is_empty()
        || matches!(trimmed, "." | "..")
        || windows_trimmed.is_empty()
        || windows_trimmed != trimmed
        || trimmed.len() > 80
        || trimmed.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    ':' | '*' | '?' | '"' | '<' | '>' | '|' | '$' | '%'
                )
        })
    {
        return false;
    }
    !matches!(
        trimmed.trim_end_matches('.').to_lowercase().as_str(),
        "con"
            | "prn"
            | "aux"
            | "nul"
            | "com1"
            | "com2"
            | "com3"
            | "com4"
            | "lpt1"
            | "lpt2"
            | "lpt3"
    )
}

fn canonical_install_route(parts: &[String]) -> Option<(String, String)> {
    let lower = parts
        .iter()
        .map(|part| part.to_lowercase())
        .collect::<Vec<_>>();
    if let Some(index) = lower
        .windows(2)
        .position(|parts| parts[0] == "content" && parts[1] == "paks")
    {
        let mut route = parts[..=index + 1].to_vec();
        route.push("~mods".to_string());
        return Some((route.join("/"), "unreal-pak".to_string()));
    }
    if let Some(index) = lower
        .windows(2)
        .position(|parts| matches!(parts[0].as_str(), "script" | "scripts") && parts[1] == "mods")
    {
        return Some((parts[..=index + 1].join("/"), "script-files".to_string()));
    }
    if let Some(index) = lower.windows(4).position(|parts| {
        parts[0] == "binaries" && parts[1] == "win64" && parts[2] == "ue4ss" && parts[3] == "mods"
    }) {
        return Some((parts[..=index + 3].join("/"), "ue4ss".to_string()));
    }
    if let Some(index) = lower
        .windows(3)
        .position(|parts| parts[0] == "binaries" && parts[1] == "win64" && parts[2] == "mods")
    {
        return Some((parts[..=index + 2].join("/"), "ue4ss".to_string()));
    }
    if let Some(index) = lower.windows(2).position(|parts| {
        parts[0] == "bepinex" && matches!(parts[1].as_str(), "plugins" | "config")
    }) {
        return Some((parts[..=index + 1].join("/"), "bepinex".to_string()));
    }
    if let Some(index) = lower.windows(2).position(|parts| {
        parts[0] == "reframework" && matches!(parts[1].as_str(), "autorun" | "plugins")
    }) {
        return Some((parts[..=index + 1].join("/"), "reframework".to_string()));
    }
    None
}

fn validate_learned_route_shape(adapter_id: &str, relative_path: &str) -> bool {
    let parts = normalize_archive_path(relative_path)
        .split('/')
        .filter(|part| !part.is_empty())
        .map(|part| part.to_lowercase())
        .collect::<Vec<_>>();
    if parts.is_empty() || parts.len() > 12 || parts.iter().any(|part| part == "..") {
        return false;
    }
    match adapter_id {
        "unreal-pak" => {
            parts.len() >= 3
                && parts[parts.len() - 3] == "content"
                && parts[parts.len() - 2] == "paks"
                && parts[parts.len() - 1] == "~mods"
        }
        "script-files" => {
            parts.len() >= 2
                && matches!(parts[parts.len() - 2].as_str(), "script" | "scripts")
                && parts[parts.len() - 1] == "mods"
        }
        "ue4ss" => {
            let legacy = parts.len() >= 3
                && parts[parts.len() - 3] == "binaries"
                && parts[parts.len() - 2] == "win64"
                && parts[parts.len() - 1] == "mods";
            let nested = parts.len() >= 4
                && parts[parts.len() - 4] == "binaries"
                && parts[parts.len() - 3] == "win64"
                && parts[parts.len() - 2] == "ue4ss"
                && parts[parts.len() - 1] == "mods";
            legacy || nested
        }
        "bepinex" => {
            parts.len() >= 2
                && parts[parts.len() - 2] == "bepinex"
                && matches!(parts[parts.len() - 1].as_str(), "plugins" | "config")
        }
        "reframework" => {
            parts.len() >= 2
                && parts[parts.len() - 2] == "reframework"
                && matches!(parts[parts.len() - 1].as_str(), "autorun" | "plugins")
        }
        _ => false,
    }
}

fn route_has_safe_internal_root(
    profile: &GameProfile,
    relative_path: &str,
    adapter_id: &str,
) -> bool {
    let Some(first) = normalize_archive_path(relative_path)
        .split('/')
        .find(|part| !part.is_empty())
        .map(str::to_string)
    else {
        return false;
    };
    let lower = first.to_lowercase();
    if matches!(
        lower.as_str(),
        "users"
            | "windows"
            | "programdata"
            | "appdata"
            | "documents"
            | "desktop"
            | "downloads"
            | "steamapps"
    ) {
        return false;
    }
    if Path::new(&profile.game_path).join(&first).exists() {
        return true;
    }
    matches!(
        (adapter_id, lower.as_str()),
        ("unreal-pak", "content")
            | ("script-files", "script")
            | ("script-files", "scripts")
            | ("ue4ss", "binaries")
            | ("bepinex", "bepinex")
            | ("reframework", "reframework")
    )
}

fn install_route_scopes(context: &str) -> Vec<String> {
    let lower = context.to_lowercase();
    let mut scopes = Vec::new();
    if lower.contains("single player") || lower.contains("single-player") || lower.contains("solo")
    {
        scopes.push("client".to_string());
    }
    if lower.contains("multiplayer")
        || lower.contains("local server")
        || lower.contains("hosting")
        || lower.contains("windowsserver")
    {
        scopes.push("hosted-server".to_string());
    }
    if lower.contains("dedicated server") || lower.contains("dedicated-server") {
        scopes.push("dedicated-server".to_string());
    }
    if scopes.is_empty() {
        scopes.push("general".to_string());
    }
    scopes
}

fn compact_route_excerpt(context: &str) -> String {
    context
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(240)
        .collect()
}

fn route_adapter_compatible(profile: &GameProfile, adapter_id: &str) -> bool {
    if profile
        .game_id
        .as_deref()
        .and_then(game_definition_by_id)
        .is_some_and(|definition| {
            definition
                .supported_adapters
                .iter()
                .any(|adapter| adapter.eq_ignore_ascii_case(adapter_id))
        })
    {
        return true;
    }
    match adapter_id {
        "bepinex" => profile.engine.starts_with("unity") || profile.loader.starts_with("bepinex"),
        "unreal-pak" | "script-files" | "ue4ss" => {
            profile.engine == "unreal" || profile.loader == "ue4ss"
        }
        "reframework" => profile.engine == "re-engine" || profile.loader == "reframework",
        _ => false,
    }
}

fn apply_profile_route_knowledge(
    profile: &GameProfile,
    knowledge: &mut ProfileRouteKnowledge,
) -> RouteKnowledgeOutcome {
    let mut outcome = RouteKnowledgeOutcome::default();
    let mut normalized_routes = Vec::new();
    for mut route in std::mem::take(&mut knowledge.routes) {
        let Some((relative_path, adapter_id)) =
            normalize_install_route_candidate(profile, &route.relative_path)
        else {
            outcome.warnings.push(format!(
                "Skipped an unsafe or unrecognized learned route: {}.",
                route.relative_path
            ));
            continue;
        };
        route.relative_path = relative_path;
        route.adapter_id = adapter_id;
        merge_learned_route(&mut normalized_routes, route);
    }
    knowledge.routes = normalized_routes;

    for route in knowledge.routes.iter_mut().filter(|route| route.trusted) {
        if !validate_learned_route_shape(&route.adapter_id, &route.relative_path)
            || !route_has_safe_internal_root(profile, &route.relative_path, &route.adapter_id)
            || !(route_adapter_compatible(profile, &route.adapter_id)
                || route.package_verified
                || profile.engine == "unknown")
        {
            outcome.warnings.push(format!(
                "Skipped an unsafe or incompatible learned route: {}.",
                route.relative_path
            ));
            continue;
        }
        push_unique_route(&mut outcome.expected_routes, &route.relative_path);
        match ensure_directory_beneath_root(Path::new(&profile.game_path), &route.relative_path) {
            Ok(created) => {
                route.created |= created;
                if created {
                    push_unique_route(&mut outcome.created_routes, &route.relative_path);
                }
            }
            Err(error) => outcome.warnings.push(format!(
                "Could not prepare learned install route {}: {}",
                route.relative_path, error
            )),
        }
    }
    outcome
}

fn ensure_directory_beneath_root(root: &Path, relative_path: &str) -> Result<bool, String> {
    if !root.is_dir() {
        return Err("The verified Steam game folder no longer exists.".to_string());
    }
    let root_canonical = fs::canonicalize(root).map_err(error_to_string)?;
    let target = safe_join(root, relative_path)?;
    if target.exists() && !target.is_dir() {
        return Err("The expected route exists as a file.".to_string());
    }

    let mut existing_ancestor = target.as_path();
    while !existing_ancestor.exists() {
        existing_ancestor = existing_ancestor
            .parent()
            .ok_or_else(|| "The route has no safe existing parent.".to_string())?;
    }
    let ancestor_canonical = fs::canonicalize(existing_ancestor).map_err(error_to_string)?;
    if !ancestor_canonical.starts_with(&root_canonical) {
        return Err("The route escapes the verified Steam game folder.".to_string());
    }
    if target.is_dir() {
        let target_canonical = fs::canonicalize(&target).map_err(error_to_string)?;
        if !target_canonical.starts_with(&root_canonical) {
            return Err("The route resolves outside the verified Steam game folder.".to_string());
        }
        return Ok(false);
    }

    fs::create_dir_all(&target).map_err(error_to_string)?;
    let target_canonical = fs::canonicalize(&target).map_err(error_to_string)?;
    if !target_canonical.starts_with(&root_canonical) {
        return Err(
            "The created route resolves outside the verified Steam game folder.".to_string(),
        );
    }
    Ok(true)
}

fn prepare_package_install_routes(
    root: &Path,
    profile: &GameProfile,
    scanned: &ScannedArchive,
    document: &ProviderRouteDocument,
) -> RouteKnowledgeOutcome {
    let candidates = extract_install_route_candidates(profile, &document.text)
        .into_iter()
        .filter(|candidate| archive_supports_adapter(scanned, &candidate.adapter_id))
        .filter(|candidate| {
            route_adapter_compatible(profile, &candidate.adapter_id) || profile.engine == "unknown"
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return RouteKnowledgeOutcome::default();
    }

    let mut knowledge = read_profile_route_knowledge(root, &profile.id)
        .ok()
        .flatten()
        .unwrap_or_else(|| ProfileRouteKnowledge {
            version: PROFILE_ROUTE_KNOWLEDGE_VERSION,
            profile_id: profile.id.clone(),
            learned_at: now_string(),
            sampled_mods: 0,
            providers: Vec::new(),
            routes: Vec::new(),
            warnings: Vec::new(),
        });
    push_unique_string_value(&mut knowledge.providers, &document.provider);

    for candidate in candidates {
        merge_learned_route(
            &mut knowledge.routes,
            LearnedInstallRoute {
                relative_path: candidate.relative_path,
                adapter_id: candidate.adapter_id,
                scopes: candidate.scopes,
                confidence: 0.92,
                supporting_mods: 1,
                providers: vec![document.provider.clone()],
                evidence: vec![RouteEvidence {
                    provider: document.provider.clone(),
                    mod_id: document.mod_id.clone(),
                    mod_name: document.mod_name.clone(),
                    excerpt: candidate.excerpt,
                }],
                trusted: true,
                package_verified: true,
                created: false,
            },
        );
    }
    knowledge.learned_at = now_string();
    let mut outcome = apply_profile_route_knowledge(profile, &mut knowledge);
    if let Err(error) = write_profile_route_knowledge(root, &knowledge) {
        outcome.warnings.push(format!(
            "Could not remember this package's install route: {error}"
        ));
    }
    outcome
}

fn archive_supports_adapter(scanned: &ScannedArchive, adapter_id: &str) -> bool {
    scanned.entries.iter().any(|entry| {
        if entry.is_directory {
            return false;
        }
        let path = entry.logical_path.to_lowercase();
        let name = basename(&path).to_lowercase();
        match adapter_id {
            "unreal-pak" => {
                path.ends_with(".pak") || path.ends_with(".ucas") || path.ends_with(".utoc")
            }
            "script-files" => path.ends_with(".as"),
            "ue4ss" => {
                path.ends_with(".lua")
                    || path.starts_with("mods/")
                    || path.starts_with("ue4ss/")
                    || is_ue4ss_root_runtime_file(&path)
            }
            "bepinex" => {
                path.contains("bepinex/")
                    || path.starts_with("plugins/")
                    || name.ends_with(".dll")
                    || is_bepinex_root_runtime_file(&path)
            }
            "reframework" => {
                path.starts_with("reframework/")
                    || path.ends_with(".lua")
                    || is_reframework_root_runtime_file(&path)
            }
            _ => false,
        }
    })
}

fn description_runtime_dependencies(
    profile: &GameProfile,
    text: &str,
    provided_runtime: Option<&str>,
) -> Vec<DependencySpec> {
    let searchable = ascii_requirement_text(text);
    if searchable.is_empty() {
        return Vec::new();
    }
    let mut dependencies = Vec::new();
    for definition in runtime_definitions() {
        if provided_runtime.is_some_and(|runtime| runtime.eq_ignore_ascii_case(&definition.id))
            || !runtime_definition_matches_profile(definition, profile)
            || !runtime_alias_is_required(&searchable, &runtime_requirement_aliases(&definition.id))
        {
            continue;
        }
        let dependency = known_runtime_dependency(profile, &definition.id);
        if !dependencies.iter().any(|existing: &DependencySpec| {
            dependency_key(existing) == dependency_key(&dependency)
        }) {
            dependencies.push(dependency);
        }
    }
    dependencies
}

fn runtime_definition_matches_profile(
    definition: &RuntimeDefinition,
    profile: &GameProfile,
) -> bool {
    profile.engine == "unknown"
        || definition
            .profile_engines
            .iter()
            .any(|engine| engine.eq_ignore_ascii_case(&profile.engine))
        || definition
            .profile_loaders
            .iter()
            .any(|loader| loader.eq_ignore_ascii_case(&profile.loader))
}

fn runtime_requirement_aliases(runtime_id: &str) -> Vec<&'static str> {
    match runtime_id {
        "bepinex" | "bepinex-il2cpp" => vec!["bepinex", "bep in ex"],
        "ue4ss" => vec!["ue4ss", "re ue4ss"],
        "reframework" => vec!["reframework", "re framework"],
        _ => Vec::new(),
    }
}

fn ascii_requirement_text(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn runtime_alias_is_required(searchable: &str, aliases: &[&str]) -> bool {
    const REQUIREMENT_WORDS: [&str; 10] = [
        "require",
        "requires",
        "required",
        "requirement",
        "requirements",
        "dependency",
        "dependencies",
        "prerequisite",
        "needs",
        "install first",
    ];
    aliases.iter().any(|alias| {
        let mut offset = 0usize;
        while let Some(relative_index) = searchable[offset..].find(alias) {
            let index = offset + relative_index;
            let alias_end = index + alias.len();
            let starts_on_boundary =
                index == 0 || !searchable.as_bytes()[index - 1].is_ascii_alphanumeric();
            let ends_on_boundary = alias_end == searchable.len()
                || !searchable.as_bytes()[alias_end].is_ascii_alphanumeric();
            if !starts_on_boundary || !ends_on_boundary {
                offset = index + 1;
                continue;
            }
            let start = index.saturating_sub(140);
            let end = (alias_end + 140).min(searchable.len());
            let local_start = index.saturating_sub(64);
            let local_end = (alias_end + 64).min(searchable.len());
            let local = &searchable[local_start..local_end];
            if !runtime_requirement_is_negated(local, alias)
                && REQUIREMENT_WORDS
                    .iter()
                    .any(|word| searchable[start..end].contains(word))
            {
                return true;
            }
            offset = alias_end;
        }
        false
    })
}

fn runtime_requirement_is_negated(context: &str, alias: &str) -> bool {
    [
        format!("does not require {alias}"),
        format!("doesnt require {alias}"),
        format!("do not require {alias}"),
        format!("not require {alias}"),
        format!("without {alias}"),
        format!("no need for {alias}"),
        format!("{alias} is not required"),
        format!("{alias} not required"),
        format!("{alias} is optional"),
        format!("{alias} optional"),
        format!("optional {alias}"),
    ]
    .iter()
    .any(|phrase| context.contains(phrase))
}

fn append_unique_dependencies(
    destination: &mut Vec<DependencySpec>,
    dependencies: impl IntoIterator<Item = DependencySpec>,
) {
    for dependency in dependencies {
        if !destination
            .iter()
            .any(|existing| dependency_key(existing) == dependency_key(&dependency))
        {
            destination.push(dependency);
        }
    }
}

fn runtime_id_for_provider_package(
    profile: &GameProfile,
    provider: &str,
    namespace: Option<&str>,
    package_name: &str,
) -> Option<String> {
    let engine_known = profile.engine != "unknown";
    let loader_known = profile.loader != "none";
    let mut candidates = runtime_definitions()
        .iter()
        .filter(|definition| {
            runtime_provider_package_matches(definition, provider, namespace, package_name)
        })
        .filter(|definition| {
            (!engine_known
                || definition
                    .profile_engines
                    .iter()
                    .any(|engine| engine.eq_ignore_ascii_case(&profile.engine)))
                && (!loader_known
                    || definition
                        .profile_loaders
                        .iter()
                        .any(|loader| loader.eq_ignore_ascii_case(&profile.loader)))
        })
        .map(|definition| definition.id.clone())
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.dedup();
    (candidates.len() == 1).then(|| candidates.remove(0))
}

fn provider_slug_aliases(slug: &str) -> Vec<String> {
    match slug {
        "dragonwilds" | "rsdragonwilds" => vec!["runescapedragonwilds".to_string()],
        "mhwilds" => vec!["monsterhunterwilds".to_string()],
        _ => Vec::new(),
    }
}

fn provider_name_aliases(slug: &str) -> Vec<String> {
    match slug {
        "dragonwilds" | "rsdragonwilds" | "runescapedragonwilds" => {
            vec!["RuneScape: Dragonwilds".to_string()]
        }
        "mhwilds" | "monsterhunterwilds" => vec!["Monster Hunter Wilds".to_string()],
        _ => Vec::new(),
    }
}

fn push_provider_slug(candidates: &mut Vec<String>, seen: &mut HashSet<String>, value: String) {
    let trimmed = value.trim_matches('-').to_string();
    if trimmed.len() >= 2 && seen.insert(trimmed.clone()) {
        candidates.push(trimmed);
    }
}

fn push_unique_string(values: &mut Vec<String>, seen: &mut HashSet<String>, value: String) {
    let trimmed = value.trim().to_string();
    let key = trimmed.to_lowercase();
    if !trimmed.is_empty() && seen.insert(key) {
        values.push(trimmed);
    }
}

fn readable_provider_text(value: &str) -> String {
    let mut output = String::new();
    let characters = value.chars().collect::<Vec<_>>();

    for (index, character) in characters.iter().enumerate() {
        let previous = index
            .checked_sub(1)
            .and_then(|previous| characters.get(previous));
        let next = characters.get(index + 1);
        let should_split_camel = character.is_ascii_uppercase()
            && previous
                .map(|previous| previous.is_ascii_lowercase() || previous.is_ascii_digit())
                .unwrap_or(false)
            && next.map(|next| next.is_ascii_lowercase()).unwrap_or(false);

        if should_split_camel && !output.ends_with(' ') {
            output.push(' ');
        }

        if character.is_ascii_alphanumeric() {
            output.push(*character);
        } else if !output.ends_with(' ') {
            output.push(' ');
        }
    }

    output.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn compact_provider_slug(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(|character| character.to_lowercase())
        .collect::<String>()
}

fn hyphenated_provider_slug(value: &str) -> String {
    readable_provider_text(value)
        .split_whitespace()
        .map(|part| {
            part.chars()
                .filter(|character| character.is_ascii_alphanumeric())
                .flat_map(|character| character.to_lowercase())
                .collect::<String>()
        })
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn discover_thunderstore_community_mods(
    store_root: &Path,
    profile: &GameProfile,
    community: &str,
) -> Result<Vec<OnlineModRecord>, String> {
    let packages = fetch_thunderstore_community_packages(community)?;
    let installed_keys = installed_package_identity_keys(store_root, &profile.id)?;

    let mut records = packages
        .into_iter()
        .filter(is_discoverable_thunderstore_package)
        .filter_map(|package| {
            thunderstore_package_to_online_mod(profile, community, package, &installed_keys)
        })
        .collect::<Vec<_>>();

    records.sort_by(|first, second| {
        second
            .downloads
            .cmp(&first.downloads)
            .then_with(|| second.rating_score.cmp(&first.rating_score))
            .then_with(|| first.name.to_lowercase().cmp(&second.name.to_lowercase()))
    });
    Ok(records)
}

fn fetch_thunderstore_community_packages(
    community: &str,
) -> Result<Vec<ThunderstoreCommunityPackage>, String> {
    let cache = THUNDERSTORE_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let cached_packages = cache
        .lock()
        .ok()
        .and_then(|cache| cache.get(community).cloned())
        .filter(|entry| entry.fetched_at.elapsed() < Duration::from_secs(10 * 60));
    let packages = if let Some(entry) = cached_packages {
        entry.packages
    } else {
        let client = thunderstore_client()?;
        let url = format!(
            "{}/{}/api/v1/package/",
            THUNDERSTORE_COMMUNITY_API_BASE,
            sanitize_url_path_segment(community)
        );
        let packages = client
            .get(url)
            .send()
            .map_err(error_to_string)?
            .error_for_status()
            .map_err(error_to_string)?
            .json::<Vec<ThunderstoreCommunityPackage>>()
            .map_err(error_to_string)?;
        if let Ok(mut cache) = cache.lock() {
            cache.insert(
                community.to_string(),
                ThunderstoreCacheEntry {
                    fetched_at: Instant::now(),
                    packages: packages.clone(),
                },
            );
        }
        packages
    };
    Ok(packages)
}

fn is_discoverable_thunderstore_package(package: &ThunderstoreCommunityPackage) -> bool {
    if package.is_deprecated || package.has_nsfw_content || package.versions.is_empty() {
        return false;
    }

    let lower_name = package.name.to_lowercase();
    let is_tool = package
        .categories
        .iter()
        .any(|category| category.eq_ignore_ascii_case("tools"))
        || matches!(lower_name.as_str(), "r2modman" | "thunderstore_mod_manager");

    !is_tool
}

fn thunderstore_package_to_online_mod(
    profile: &GameProfile,
    community: &str,
    package: ThunderstoreCommunityPackage,
    installed_keys: &HashSet<String>,
) -> Option<OnlineModRecord> {
    let latest = latest_active_thunderstore_version(&package)?.clone();
    let package_ref = ThunderstorePackageRef {
        namespace: package.owner.clone(),
        name: package.name.clone(),
        version: Some(latest.version_number.clone()),
    };
    let package_id = thunderstore_package_id(&package_ref);
    let dependency_string = thunderstore_dependency_string(&package_ref, &latest.version_number);
    let installed = installed_keys.contains(&format!("package:{}", package_id.to_lowercase()))
        || installed_keys.contains(&format!("dependency:{}", dependency_string.to_lowercase()));
    let total_downloads = package
        .versions
        .iter()
        .map(|version| version.downloads)
        .sum::<u64>();
    let created_at = package
        .date_created
        .clone()
        .or_else(|| latest.date_created.clone());
    let updated_at = package
        .date_updated
        .clone()
        .or_else(|| latest.date_created.clone());

    Some(OnlineModRecord {
        id: package_id,
        provider: "thunderstore".to_string(),
        provider_label: "Thunderstore".to_string(),
        game_id: profile.game_id.clone(),
        provider_game_id: Some(community.to_string()),
        name: humanize_mod_display_name(&package.full_name),
        owner: package.owner,
        version: latest.version_number,
        description: latest.description,
        categories: package.categories,
        downloads: total_downloads,
        rating_score: package.rating_score,
        dependency_count: latest.dependencies.len(),
        file_size: latest.file_size,
        icon_url: latest.icon,
        package_url: package.package_url,
        website_url: latest.website_url,
        installed,
        created_at,
        updated_at,
        install_supported: true,
        install_note: None,
    })
}

fn installed_package_identity_keys(
    store_root: &Path,
    profile_id: &str,
) -> Result<HashSet<String>, String> {
    let store = read_store::<InstalledModRecord>(&installed_mods_path(store_root))
        .map_err(error_to_string)?;
    let mut keys = HashSet::new();
    for record in store
        .items
        .iter()
        .filter(|record| record.profile_id == profile_id && record.enabled)
    {
        if let Some(package_id) = &record.package_id {
            keys.insert(format!("package:{}", package_id.to_lowercase()));
        }
        if let Some(dependency) = &record.dependency_string {
            keys.insert(format!("dependency:{}", dependency.to_lowercase()));
        }
    }
    Ok(keys)
}

fn discover_nexus_mods_for_profile(
    profile: &GameProfile,
    domain: &str,
    nexus_api_ready: bool,
    max_results: usize,
    sort: &str,
) -> Result<(Vec<OnlineModRecord>, usize), String> {
    let client = provider_client()?;
    let mut records = Vec::new();
    let mut offset = 0usize;
    let mut total_count = usize::MAX;

    while offset < total_count && records.len() < max_results {
        let remaining = max_results.saturating_sub(records.len());
        let page_size = nexus_discovery_batch_size(remaining);
        if page_size == 0 {
            break;
        }

        let page = fetch_nexus_mod_page_sorted(&client, domain, offset, page_size, sort)?;
        total_count = page.total_count;
        let returned_count = page.nodes.len();

        if returned_count == 0 {
            break;
        }

        for node in page.nodes {
            if let Some(record) = nexus_mod_to_online_mod(profile, domain, node, nexus_api_ready) {
                records.push(record);
            }
        }

        offset += returned_count;
    }

    Ok((records, total_count.min(usize::MAX - 1)))
}

fn nexus_discovery_batch_size(remaining: usize) -> usize {
    remaining.min(NEXUS_DISCOVERY_PAGE_SIZE)
}

fn fetch_nexus_mod_page_sorted(
    client: &Client,
    domain: &str,
    offset: usize,
    count: usize,
    sort: &str,
) -> Result<NexusModPage, String> {
    fetch_nexus_mod_page_with_sort(client, domain, offset, count, sort)
}

fn fetch_nexus_mod_page(
    client: &Client,
    domain: &str,
    offset: usize,
    count: usize,
) -> Result<NexusModPage, String> {
    fetch_nexus_mod_page_with_sort(client, domain, offset, count, "downloads")
}

fn fetch_nexus_mod_page_with_sort(
    client: &Client,
    domain: &str,
    offset: usize,
    count: usize,
    sort: &str,
) -> Result<NexusModPage, String> {
    const NEXUS_DISCOVERY_QUERY: &str = r#"
        query UniLoaderDiscoverMods(
          $filter: ModsFilter,
          $sort: [ModsSort!],
          $offset: Int,
          $count: Int
        ) {
          mods(filter: $filter, sort: $sort, offset: $offset, count: $count) {
            totalCount
            nodes {
              id
              modId
              name
              summary
              author
              category
              version
              createdAt
              updatedAt
              downloads
              endorsements
              fileSize
              pictureUrl
              thumbnailUrl
              thumbnailLargeUrl
              directDownloadEnabled
              adultContent
              status
              modRequirements(skipDisabledRequirements: true) {
                nexusRequirements {
                  totalCount
                  nodes {
                    externalRequirement
                    gameId
                    modId
                    modName
                    notes
                    url
                  }
                }
              }
            }
          }
        }
    "#;

    let sort_value = match sort {
        "newest" => serde_json::json!({ "updatedAt": { "direction": "DESC" } }),
        "oldest" => serde_json::json!({ "createdAt": { "direction": "ASC" } }),
        _ => serde_json::json!({ "downloads": { "direction": "DESC" } }),
    };
    let body = serde_json::json!({
        "query": NEXUS_DISCOVERY_QUERY,
        "variables": {
            "filter": {
                "op": "AND",
                "gameDomainName": [{ "value": domain, "op": "EQUALS" }],
                "adultContent": [{ "value": false, "op": "EQUALS" }],
                "status": [{ "value": "published", "op": "EQUALS" }]
            },
            "sort": [sort_value],
            "offset": offset,
            "count": count
        }
    });

    let response = client
        .post(NEXUS_GRAPHQL_API_BASE)
        .json(&body)
        .send()
        .map_err(error_to_string)?
        .error_for_status()
        .map_err(error_to_string)?
        .json::<NexusGraphqlResponse>()
        .map_err(error_to_string)?;

    if !response.errors.is_empty() {
        let messages = response
            .errors
            .into_iter()
            .map(|error| error.message)
            .collect::<Vec<_>>()
            .join("; ");
        return Err(messages);
    }

    response
        .data
        .map(|data| data.mods)
        .ok_or_else(|| "Nexus Mods returned no discovery data.".to_string())
}

fn fetch_nexus_game_domain_by_name(client: &Client, name: &str) -> Option<String> {
    let nodes = fetch_nexus_games_by_name(client, name)?;
    let normalized_name = compact_provider_slug(name);
    nodes.into_iter().find_map(|node| {
        let node_name = compact_provider_slug(&node.name);
        let node_domain = compact_provider_slug(&node.domain_name);
        if node_name == normalized_name || node_domain == normalized_name {
            Some(node.domain_name)
        } else {
            None
        }
    })
}

fn fetch_nexus_games_by_name(client: &Client, name: &str) -> Option<Vec<NexusGameNode>> {
    const NEXUS_GAME_QUERY: &str = r#"
        query UniLoaderDiscoverGame(
          $filter: GamesSearchFilter,
          $sort: [GamesSearchSort!],
          $offset: Int,
          $count: Int
        ) {
          games(filter: $filter, sort: $sort, offset: $offset, count: $count) {
            nodes {
              id
              name
              domainName
            }
          }
        }
    "#;

    let body = serde_json::json!({
        "query": NEXUS_GAME_QUERY,
        "variables": {
            "filter": {
                "name": [{ "value": name, "op": "MATCHES" }]
            },
            "sort": [{ "relevance": { "direction": "DESC" } }],
            "offset": 0,
            "count": 5
        }
    });

    let response = client
        .post(NEXUS_GRAPHQL_API_BASE)
        .json(&body)
        .send()
        .ok()?
        .error_for_status()
        .ok()?
        .json::<NexusGamesGraphqlResponse>()
        .ok()?;

    if !response.errors.is_empty() {
        return None;
    }

    Some(response.data?.games.nodes)
}

fn nexus_game_lookup_names(profile: &GameProfile, domain: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut seen = HashSet::new();

    for alias in provider_name_aliases(&compact_provider_slug(domain)) {
        push_unique_string(&mut names, &mut seen, alias);
    }
    for name in provider_name_candidates(profile) {
        push_unique_string(&mut names, &mut seen, name);
    }
    push_unique_string(&mut names, &mut seen, profile.name.clone());
    push_unique_string(&mut names, &mut seen, readable_provider_text(domain));
    names
}

fn fetch_nexus_game_id_for_domain(
    client: &Client,
    profile: &GameProfile,
    domain: &str,
    api_key: Option<&str>,
) -> Result<u64, String> {
    let normalized_domain = compact_provider_slug(domain);
    if let Ok(cache) = NEXUS_GAME_ID_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        if let Some(game_id) = cache.get(&normalized_domain) {
            return Ok(*game_id);
        }
    }

    if let Some(api_key) = api_key {
        let direct_url = format!(
            "https://api.nexusmods.com/v1/games/{}.json",
            sanitize_url_path_segment(domain)
        );
        if let Ok(response) = nexus_api_get(client, &direct_url, api_key).send() {
            if let Ok(response) = response.error_for_status() {
                if let Ok(details) = response.json::<NexusGameDetails>() {
                    let returned_domain = compact_provider_slug(&details.domain_name);
                    if returned_domain.is_empty() || returned_domain == normalized_domain {
                        cache_nexus_game_id(&normalized_domain, details.id);
                        return Ok(details.id);
                    }
                }
            }
        }
    }

    for name in nexus_game_lookup_names(profile, domain) {
        let Some(nodes) = fetch_nexus_games_by_name(client, &name) else {
            continue;
        };
        if let Some(game_id) = nodes
            .into_iter()
            .find(|node| compact_provider_slug(&node.domain_name) == normalized_domain)
            .and_then(|node| node.id)
        {
            cache_nexus_game_id(&normalized_domain, game_id);
            return Ok(game_id);
        }
    }

    Err(format!(
        "Nexus Mods could not resolve the game catalogue id for '{domain}'. Refresh Discovery and try again."
    ))
}

fn cache_nexus_game_id(domain: &str, game_id: u64) {
    if let Ok(mut cache) = NEXUS_GAME_ID_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        cache.insert(domain.to_string(), game_id);
    }
}

fn fetch_nexus_mod_requirements(
    client: &Client,
    profile: &GameProfile,
    domain: &str,
    mod_id: u64,
    api_key: Option<&str>,
) -> Result<Vec<NexusRequirement>, String> {
    const NEXUS_REQUIREMENTS_QUERY: &str = r#"
        query UniLoaderModRequirements($filter: ModsFilter, $count: Int) {
          mods(filter: $filter, count: $count) {
            nodes {
              modId
              modRequirements(skipDisabledRequirements: true) {
                nexusRequirements {
                  totalCount
                  nodes {
                    externalRequirement
                    gameId
                    modId
                    modName
                    notes
                    url
                  }
                }
              }
            }
          }
        }
    "#;
    let game_id = fetch_nexus_game_id_for_domain(client, profile, domain, api_key)?;
    let body = serde_json::json!({
        "query": NEXUS_REQUIREMENTS_QUERY,
        "variables": {
            "filter": {
                "op": "AND",
                "gameId": [{ "value": game_id.to_string(), "op": "EQUALS" }],
                "modId": [{ "value": mod_id.to_string(), "op": "EQUALS" }]
            },
            "count": 1
        }
    });
    let response = client
        .post(NEXUS_GRAPHQL_API_BASE)
        .json(&body)
        .send()
        .map_err(error_to_string)?
        .error_for_status()
        .map_err(error_to_string)?
        .json::<NexusGraphqlResponse>()
        .map_err(error_to_string)?;
    if !response.errors.is_empty() {
        return Err(response
            .errors
            .into_iter()
            .map(|error| error.message)
            .collect::<Vec<_>>()
            .join("; "));
    }

    Ok(response
        .data
        .and_then(|data| data.mods.nodes.into_iter().next())
        .and_then(|node| node.mod_requirements)
        .map(|requirements| requirements.nexus_requirements.nodes)
        .unwrap_or_default())
}

fn nexus_requirement_dependencies(
    profile: &GameProfile,
    parent_domain: &str,
    requirements: &[NexusRequirement],
) -> Vec<DependencySpec> {
    let mut dependencies = Vec::new();
    let mut seen = HashSet::new();

    for requirement in requirements {
        let required = !nexus_requirement_is_optional(requirement);
        let dependency =
            if let Some(runtime_id) = runtime_id_for_nexus_requirement(profile, requirement) {
                let mut dependency = known_runtime_dependency(profile, &runtime_id);
                dependency.required = required;
                dependency.notes = requirement.notes.clone().or(dependency.notes);
                dependency
            } else if !requirement.external_requirement {
                let domain = nexus_requirement_domain(requirement, parent_domain);
                let Ok(mod_id) = requirement.mod_id.trim().parse::<u64>() else {
                    continue;
                };
                DependencySpec {
                    id: format!("nexus:{domain}/{mod_id}"),
                    name: non_empty_string(requirement.mod_name.trim())
                        .unwrap_or_else(|| format!("Nexus mod {mod_id}")),
                    version: None,
                    provider: "nexus".to_string(),
                    required,
                    status: "missing".to_string(),
                    source: non_empty_string(requirement.url.trim()),
                    notes: requirement.notes.clone(),
                }
            } else {
                let source = non_empty_string(requirement.url.trim());
                let name = nexus_external_requirement_name(requirement);
                if name.is_none() && source.is_none() {
                    continue;
                }
                let identity = name
                    .as_deref()
                    .or(source.as_deref())
                    .unwrap_or("external-requirement");
                DependencySpec {
                    id: format!("external:{}", compact_provider_slug(identity)),
                    name: name.unwrap_or_else(|| "External requirement".to_string()),
                    version: None,
                    provider: "manual".to_string(),
                    required,
                    status: "missing".to_string(),
                    source,
                    notes: requirement.notes.clone(),
                }
            };
        if seen.insert(dependency_key(&dependency)) {
            dependencies.push(dependency);
        }
    }

    dependencies
}

fn runtime_id_for_nexus_requirement(
    profile: &GameProfile,
    requirement: &NexusRequirement,
) -> Option<String> {
    if let Some(runtime) =
        runtime_id_for_provider_package(profile, "nexus", None, &requirement.mod_name)
    {
        return Some(runtime);
    }

    let evidence = [
        requirement.mod_name.as_str(),
        requirement.notes.as_deref().unwrap_or_default(),
        requirement.url.as_str(),
    ]
    .into_iter()
    .filter(|value| !value.trim().is_empty())
    .collect::<Vec<_>>()
    .join(" ");
    let searchable = ascii_requirement_text(&minimal_url_decode(&evidence));
    if searchable.is_empty() {
        return None;
    }

    let mut candidates = runtime_definitions()
        .iter()
        .filter(|definition| runtime_definition_matches_profile(definition, profile))
        .filter(|definition| {
            runtime_definition_matches_requirement_evidence(definition, &searchable)
        })
        .map(|definition| definition.id.clone())
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.dedup();
    (candidates.len() == 1).then(|| candidates.remove(0))
}

fn runtime_definition_matches_requirement_evidence(
    definition: &RuntimeDefinition,
    searchable: &str,
) -> bool {
    let mut aliases = vec![definition.id.clone(), definition.dependency.name.clone()];
    aliases.extend(
        runtime_requirement_aliases(&definition.id)
            .into_iter()
            .map(str::to_string),
    );
    for package in definition
        .provider_packages
        .iter()
        .filter(|package| package.provider.eq_ignore_ascii_case("nexus"))
    {
        aliases.extend(package.package_patterns.iter().cloned());
    }

    aliases.into_iter().any(|alias| {
        let normalized = ascii_requirement_text(alias.trim_matches(['*', '?', ' ']));
        !normalized.is_empty() && normalized_phrase_is_present(searchable, &normalized)
    })
}

fn normalized_phrase_is_present(searchable: &str, phrase: &str) -> bool {
    let haystack = format!(" {searchable} ");
    let needle = format!(" {phrase} ");
    haystack.contains(&needle)
}

fn nexus_external_requirement_name(requirement: &NexusRequirement) -> Option<String> {
    non_empty_string(requirement.mod_name.trim()).or_else(|| {
        let parsed = url::Url::parse(requirement.url.trim()).ok()?;
        let filename = parsed
            .path_segments()?
            .rfind(|segment| !segment.is_empty())?;
        let decoded = minimal_url_decode(filename);
        let stem = Path::new(&decoded)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(decoded.as_str());
        non_empty_string(humanize_mod_display_name(stem).trim())
    })
}

fn nexus_requirement_domain(requirement: &NexusRequirement, parent_domain: &str) -> String {
    if let Ok(url) = url::Url::parse(requirement.url.trim()) {
        let segments = url
            .path_segments()
            .map(|segments| {
                segments
                    .filter(|segment| !segment.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let domain_index = usize::from(segments.first().is_some_and(|segment| *segment == "games"));
        if segments
            .get(domain_index + 1)
            .is_some_and(|segment| *segment == "mods")
        {
            if let Some(domain) = segments.get(domain_index) {
                return sanitize_url_path_segment(domain);
            }
        }
    }

    let provider_game_id = requirement.game_id.trim();
    if provider_game_id
        .chars()
        .any(|character| character.is_ascii_alphabetic())
    {
        return sanitize_url_path_segment(provider_game_id);
    }

    parent_domain.to_string()
}

fn nexus_requirement_is_optional(requirement: &NexusRequirement) -> bool {
    let text = format!(
        "{} {}",
        requirement.mod_name,
        requirement.notes.as_deref().unwrap_or_default()
    )
    .to_lowercase();
    [
        "optional",
        "not required",
        "alternative",
        "choose one",
        "only needed if",
        "only required if",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

fn nexus_mod_to_online_mod(
    profile: &GameProfile,
    domain: &str,
    node: NexusModNode,
    nexus_api_ready: bool,
) -> Option<OnlineModRecord> {
    if node.adult_content
        || !node
            .status
            .as_deref()
            .unwrap_or("published")
            .eq_ignore_ascii_case("published")
    {
        return None;
    }

    let mod_id = node.mod_id?;
    let name = node.name?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    let dependency_count = node
        .mod_requirements
        .as_ref()
        .map(|requirements| {
            requirements
                .nexus_requirements
                .total_count
                .max(requirements.nexus_requirements.nodes.len())
        })
        .unwrap_or_default();

    let page_url = format!(
        "{}/{}/mods/{}",
        NEXUS_SITE_BASE,
        sanitize_url_path_segment(domain),
        mod_id
    );
    let category = node
        .category
        .and_then(|value| non_empty_string(value.trim()));
    let categories = category.into_iter().collect::<Vec<_>>();
    let icon_url = node
        .thumbnail_large_url
        .or(node.thumbnail_url)
        .or(node.picture_url);
    let install_note = if nexus_api_ready {
        "Choose a file to install it through UniLoader."
    } else if node.direct_download_enabled {
        "Add a Nexus API key in Settings to enable direct install for Nexus mods."
    } else {
        "Add a Nexus API key in Settings to enable Nexus installs."
    };

    Some(OnlineModRecord {
        id: format!("nexus:{}/{}", sanitize_url_path_segment(domain), mod_id),
        provider: "nexus".to_string(),
        provider_label: "Nexus Mods".to_string(),
        game_id: profile.game_id.clone(),
        provider_game_id: Some(domain.to_string()),
        name,
        owner: node
            .author
            .and_then(|value| non_empty_string(value.trim()))
            .unwrap_or_else(|| "Nexus Mods".to_string()),
        version: node
            .version
            .and_then(|value| non_empty_string(value.trim()))
            .unwrap_or_else(|| "latest".to_string()),
        description: clean_nexus_summary(node.summary),
        categories,
        downloads: node.downloads,
        rating_score: node.endorsements,
        dependency_count,
        file_size: node.file_size.map(|size_kb| size_kb.saturating_mul(1024)),
        icon_url,
        package_url: Some(page_url.clone()),
        website_url: Some(page_url),
        installed: false,
        created_at: node.created_at,
        updated_at: node.updated_at,
        install_supported: nexus_api_ready,
        install_note: Some(install_note.to_string()),
    })
}

fn clean_nexus_summary(summary: Option<String>) -> String {
    let Some(raw_summary) = summary else {
        return String::new();
    };

    let raw_summary = decode_provider_html_entities(&raw_summary);
    let mut cleaned = String::new();
    let mut skipping_html = false;
    let mut skipping_bbcode = false;

    for character in raw_summary.chars() {
        match character {
            '<' if !skipping_bbcode => skipping_html = true,
            '>' if skipping_html => skipping_html = false,
            '[' if !skipping_html => skipping_bbcode = true,
            ']' if skipping_bbcode => skipping_bbcode = false,
            _ if !skipping_html && !skipping_bbcode => cleaned.push(character),
            _ => {}
        }
    }

    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn non_empty_string(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn latest_active_thunderstore_version(
    package: &ThunderstoreCommunityPackage,
) -> Option<&ThunderstoreVersion> {
    package
        .versions
        .iter()
        .find(|version| version.is_active)
        .or_else(|| package.versions.first())
}

fn list_discovered_mod_files_impl(
    profile: &GameProfile,
    settings: &AppSettings,
    provider: &str,
    mod_id: &str,
    supplied_provider_game_id: Option<&str>,
) -> Result<Vec<OnlineModFileOption>, String> {
    match provider {
        "nexus" => list_nexus_mod_files(profile, settings, mod_id, supplied_provider_game_id),
        "thunderstore" => {
            list_thunderstore_mod_versions(profile, mod_id, supplied_provider_game_id)
        }
        _ => Err(format!(
            "{} does not expose selectable files in this build.",
            provider
        )),
    }
}

fn list_nexus_mod_files(
    profile: &GameProfile,
    settings: &AppSettings,
    mod_id: &str,
    supplied_provider_game_id: Option<&str>,
) -> Result<Vec<OnlineModFileOption>, String> {
    let api_key = settings.nexus_api_key().ok_or_else(|| {
        "Add your Nexus API key in Settings before loading this mod's files.".to_string()
    })?;
    let (domain, nexus_mod_id) = parse_nexus_online_mod_id(mod_id)?;
    verified_discovery_provider_game(profile, "nexus", supplied_provider_game_id, Some(&domain))?;

    let client = provider_client()?;
    let account = validate_nexus_api_key(api_key)?;
    let mut files = fetch_nexus_mod_files(&client, api_key, &domain, nexus_mod_id)?;
    files.retain(nexus_file_supported);
    files.sort_by(|first, second| {
        nexus_file_category_score(second)
            .cmp(&nexus_file_category_score(first))
            .then_with(|| {
                second
                    .is_primary
                    .unwrap_or(false)
                    .cmp(&first.is_primary.unwrap_or(false))
            })
            .then_with(|| {
                second
                    .uploaded_timestamp
                    .unwrap_or_default()
                    .cmp(&first.uploaded_timestamp.unwrap_or_default())
            })
    });

    Ok(files
        .into_iter()
        .map(|file| OnlineModFileOption {
            id: file.file_id.to_string(),
            name: nexus_file_display_name(&file),
            version: file
                .version
                .as_deref()
                .and_then(|value| non_empty_string(value.trim())),
            category: file
                .category_name
                .as_deref()
                .and_then(|value| non_empty_string(value.trim())),
            description: file.description.clone().and_then(|description| {
                non_empty_string(clean_nexus_summary(Some(description)).trim())
            }),
            file_name: file.file_name.clone(),
            file_size: nexus_file_size(&file),
            uploaded_at: nexus_file_uploaded_at(&file),
            primary: file.is_primary.unwrap_or(false),
            action: if account.is_premium {
                "direct".to_string()
            } else {
                "browser".to_string()
            },
            download_page_url: Some(nexus_file_page_url(&domain, nexus_mod_id, file.file_id)),
        })
        .collect())
}

fn list_thunderstore_mod_versions(
    profile: &GameProfile,
    mod_id: &str,
    supplied_provider_game_id: Option<&str>,
) -> Result<Vec<OnlineModFileOption>, String> {
    let raw_id = mod_id
        .strip_prefix("thunderstore:")
        .ok_or_else(|| format!("Invalid Thunderstore mod id: {}", mod_id))?;
    let package_ref = parse_thunderstore_token(raw_id, None).ok_or_else(|| {
        format!(
            "Could not parse Thunderstore package reference from {}.",
            mod_id
        )
    })?;
    let community = verified_discovery_provider_game(
        profile,
        "thunderstore",
        supplied_provider_game_id,
        supplied_provider_game_id,
    )?;
    let packages = fetch_thunderstore_community_packages(&community)?;
    let package = packages
        .into_iter()
        .find(|package| {
            package.owner.eq_ignore_ascii_case(&package_ref.namespace)
                && package.name.eq_ignore_ascii_case(&package_ref.name)
        })
        .ok_or_else(|| {
            format!(
                "Thunderstore no longer lists {}/{} for this game.",
                package_ref.namespace, package_ref.name
            )
        })?;
    let primary_version =
        latest_active_thunderstore_version(&package).map(|version| version.version_number.clone());
    let package_url = package.package_url.clone();
    let mut versions = package.versions;
    versions.retain(|version| version.is_active);
    versions.sort_by(|first, second| {
        second
            .date_created
            .cmp(&first.date_created)
            .then_with(|| second.version_number.cmp(&first.version_number))
    });

    Ok(versions
        .into_iter()
        .map(|version| OnlineModFileOption {
            id: version.version_number.clone(),
            name: format!("Version {}", version.version_number),
            version: Some(version.version_number.clone()),
            category: Some("Release".to_string()),
            description: non_empty_string(version.description.trim()),
            file_name: Some(format!("{}.zip", version.full_name)),
            file_size: version.file_size,
            uploaded_at: version.date_created,
            primary: primary_version.as_deref() == Some(version.version_number.as_str()),
            action: "direct".to_string(),
            download_page_url: package_url.clone(),
        })
        .collect())
}

fn nexus_file_size(file: &NexusFileRecord) -> Option<u64> {
    file.size
        .or_else(|| file.size_kb.map(|size| size.saturating_mul(1024)))
}

fn nexus_file_uploaded_at(file: &NexusFileRecord) -> Option<String> {
    file.uploaded_time.clone().or_else(|| {
        file.uploaded_timestamp.and_then(|timestamp| {
            chrono::DateTime::<Utc>::from_timestamp(timestamp as i64, 0)
                .map(|value| value.to_rfc3339())
        })
    })
}

fn nexus_file_page_url(domain: &str, mod_id: u64, file_id: u64) -> String {
    format!(
        "{}/{}/mods/{}?tab=files&file_id={}",
        NEXUS_SITE_BASE,
        sanitize_url_path_segment(domain),
        mod_id,
        file_id
    )
}

fn nexus_manager_download_page_url(domain: &str, mod_id: u64, file_id: u64) -> String {
    format!(
        "{}/{}/mods/{}?tab=files&file_id={}&nmm=1",
        NEXUS_SITE_BASE,
        sanitize_url_path_segment(domain),
        mod_id,
        file_id
    )
}

fn install_nexus_discovered_mod(
    store_root: &Path,
    profile: &GameProfile,
    mod_id: &str,
    settings: &AppSettings,
    version: Option<String>,
    supplied_provider_game_id: Option<&str>,
    selected_file_id: Option<&str>,
) -> Result<InstallResult, String> {
    let api_key = settings.nexus_api_key().ok_or_else(|| {
        "Add your Nexus API key in Settings before installing Nexus mods.".to_string()
    })?;
    let (domain, nexus_mod_id) = parse_nexus_online_mod_id(mod_id)?;
    let provider_game_id = verified_discovery_provider_game(
        profile,
        "nexus",
        supplied_provider_game_id,
        Some(&domain),
    )?;
    let client = provider_client()?;
    let account = validate_nexus_api_key(api_key)?;
    if !account.is_premium {
        return Err(
            "Your Nexus key is valid, but Nexus only gives direct API download links to Premium accounts. Open the selected file's Nexus download page and use its browser download instead."
                .to_string(),
        );
    }
    let files = fetch_nexus_mod_files(&client, api_key, &domain, nexus_mod_id)?;
    let file = choose_requested_nexus_file(&files, selected_file_id)?.ok_or_else(|| {
        "Nexus returned no installable files for this mod. Open the mod page and download manually."
            .to_string()
    })?;
    let links = fetch_nexus_download_links(&client, api_key, &domain, nexus_mod_id, file.file_id)?;
    let download_url = nexus_http_download_url(&links).ok_or_else(|| {
        "Nexus did not return a direct HTTP download link for this file. It may require opening the Nexus page or using an NXM handler."
            .to_string()
    })?;

    let mut visited_dependencies = HashSet::new();
    install_resolved_nexus_file(
        store_root,
        profile,
        &client,
        api_key,
        &account,
        &domain,
        nexus_mod_id,
        &file,
        &download_url,
        version,
        provider_game_id,
        &mut visited_dependencies,
        0,
    )
}

#[allow(clippy::too_many_arguments)]
fn install_resolved_nexus_file(
    store_root: &Path,
    profile: &GameProfile,
    client: &Client,
    api_key: &str,
    account: &NexusUserValidation,
    domain: &str,
    nexus_mod_id: u64,
    file: &NexusFileRecord,
    download_url: &str,
    version: Option<String>,
    provider_game_id: String,
    visited_dependencies: &mut HashSet<String>,
    dependency_depth: usize,
) -> Result<InstallResult, String> {
    if dependency_depth > MAX_DEPENDENCY_DEPTH {
        return Err("Nexus dependency chain is too deep to install safely.".to_string());
    }
    let visit_key = format!("nexus:{}/{}", domain.to_lowercase(), nexus_mod_id);
    if !visited_dependencies.insert(visit_key) {
        return Err(
            "Nexus returned a circular dependency chain. No duplicate package was installed."
                .to_string(),
        );
    }

    let details =
        fetch_nexus_mod_details(client, api_key, domain, nexus_mod_id).unwrap_or_default();
    let icon_url = nexus_mod_details_icon_url(&details);
    let mod_name =
        non_empty_string(details.name.trim()).unwrap_or_else(|| nexus_file_display_name(file));
    let provider_document = ProviderRouteDocument {
        provider: "Nexus Mods".to_string(),
        mod_id: format!("nexus:{domain}/{nexus_mod_id}"),
        mod_name: mod_name.clone(),
        text: [
            details.summary.as_str(),
            details.description.as_str(),
            file.description.as_deref().unwrap_or_default(),
        ]
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n"),
    };
    let provided_runtime = runtime_id_for_provider_package(profile, "nexus", None, &mod_name);
    let requirements =
        fetch_nexus_mod_requirements(client, profile, domain, nexus_mod_id, Some(api_key))?;
    let mut dependencies = nexus_requirement_dependencies(profile, domain, &requirements);
    append_unique_dependencies(
        &mut dependencies,
        description_runtime_dependencies(
            profile,
            &provider_document.text,
            provided_runtime.as_deref(),
        ),
    );
    let dependency_warnings = install_nexus_requirement_dependencies(
        store_root,
        profile,
        client,
        api_key,
        account,
        &dependencies,
        visited_dependencies,
        dependency_depth + 1,
    )?;

    let archive_path =
        download_nexus_file(store_root, client, domain, nexus_mod_id, file, download_url)?;
    let package_id = format!("nexus:{}/{}", domain, nexus_mod_id);
    let source_identity = provider_source_identity(
        "nexus",
        package_id.clone(),
        version,
        Some(provider_game_id),
        "Nexus Mods discovery result and file download",
    );
    let scanned = scan_import_source(store_root, &archive_path)?;
    let route_outcome =
        prepare_package_install_routes(store_root, profile, &scanned, &provider_document);
    let analysis = analyze_scanned_archive_with_identity(scanned, profile, Some(source_identity));
    let install_source_path = analysis.archive_path.clone();
    let package_identity = analysis.package_identity.clone();
    let mut plan = analysis
        .recommended_plan
        .ok_or_else(|| analysis.compatibility.reason.clone())?;

    append_unique_dependencies(&mut plan.dependencies, dependencies);
    plan.warnings.extend(dependency_warnings);
    plan.warnings.extend(route_outcome.warnings);

    if plan.adapter_id == "loose-files" || plan.requires_confirmation {
        return Err(format!(
            "{} downloaded, but UniLoader could not identify a safe automatic install layout.",
            nexus_file_display_name(file)
        ));
    }

    let dependency_string = Some(format!("{}#{}", package_id, file.file_id));
    install_archive_impl_with_metadata(
        store_root,
        profile,
        &install_source_path,
        &plan,
        InstallOptions {
            metadata: InstallMetadata {
                archive_name: file
                    .file_name
                    .clone()
                    .or_else(|| Some(nexus_file_display_name(file))),
                package_id: Some(package_id),
                dependency_string,
                display_name: Some(nexus_file_display_name(file)),
                package_identity: Some(package_identity),
                runtime_id: None,
                icon_url,
            },
            resolve_dependencies: true,
            visited_dependencies,
            dependency_depth,
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn install_nexus_requirement_dependencies(
    store_root: &Path,
    profile: &GameProfile,
    client: &Client,
    api_key: &str,
    account: &NexusUserValidation,
    dependencies: &[DependencySpec],
    visited_dependencies: &mut HashSet<String>,
    dependency_depth: usize,
) -> Result<Vec<String>, String> {
    if dependency_depth > MAX_DEPENDENCY_DEPTH {
        return Err("Nexus dependency chain is too deep to install safely.".to_string());
    }
    let mut warnings = Vec::new();

    for dependency in dependencies.iter().filter(|dependency| dependency.required) {
        let dependency = refresh_dependency_status(store_root, profile, dependency);
        if dependency.status == "already-installed" {
            continue;
        }

        if dependency.provider != "nexus" {
            match install_dependency_by_provider(
                store_root,
                profile,
                &dependency,
                visited_dependencies,
                dependency_depth,
            ) {
                Ok(mut installed_warnings) => warnings.append(&mut installed_warnings),
                Err(error) => {
                    return Err(format!(
                        "{} requires {}. UniLoader checked the game folder first, but the requirement is missing: {}",
                        profile_game_label(profile), dependency.name, error
                    ));
                }
            }
            continue;
        }

        let (dependency_domain, dependency_mod_id) = parse_nexus_online_mod_id(&dependency.id)?;
        verified_discovery_provider_game(
            profile,
            "nexus",
            Some(&dependency_domain),
            Some(&dependency_domain),
        )?;
        let visit_key = format!(
            "nexus:{}/{}",
            dependency_domain.to_lowercase(),
            dependency_mod_id
        );
        if visited_dependencies.contains(&visit_key) {
            continue;
        }
        if !account.is_premium {
            return Err(format!(
                "Missing required Nexus mod: {}. Confirm this dependency in UniLoader, then authorize its Slow download on Nexus before installing the parent mod.",
                dependency.name
            ));
        }

        let files = fetch_nexus_mod_files(client, api_key, &dependency_domain, dependency_mod_id)?;
        let dependency_file = choose_nexus_file(&files).ok_or_else(|| {
            format!(
                "{} is required, but Nexus returned no supported main archive for it.",
                dependency.name
            )
        })?;
        let links = fetch_nexus_download_links(
            client,
            api_key,
            &dependency_domain,
            dependency_mod_id,
            dependency_file.file_id,
        )?;
        let download_url = nexus_http_download_url(&links).ok_or_else(|| {
            format!(
                "Nexus did not return a direct download server for required mod {}.",
                dependency.name
            )
        })?;
        let result = install_resolved_nexus_file(
            store_root,
            profile,
            client,
            api_key,
            account,
            &dependency_domain,
            dependency_mod_id,
            &dependency_file,
            &download_url,
            dependency_file.version.clone(),
            dependency_domain.clone(),
            visited_dependencies,
            dependency_depth,
        )?;
        warnings.extend(result.warnings);
        warnings.push(format!("Installed required Nexus mod {}.", dependency.name));
    }

    Ok(warnings)
}

fn parse_nexus_online_mod_id(mod_id: &str) -> Result<(String, u64), String> {
    let raw = mod_id
        .strip_prefix("nexus:")
        .ok_or_else(|| format!("Invalid Nexus mod id: {}", mod_id))?;
    let (domain, id) = raw
        .split_once('/')
        .ok_or_else(|| format!("Invalid Nexus mod id: {}", mod_id))?;
    let nexus_mod_id = id
        .parse::<u64>()
        .map_err(|_| format!("Invalid Nexus mod id: {}", mod_id))?;
    Ok((sanitize_url_path_segment(domain), nexus_mod_id))
}

fn parse_nexus_nxm_link(raw: &str) -> Result<NexusNxmLink, String> {
    let invalid = || {
        "Nexus sent an invalid download handoff. Return to the file page and click Slow download again.".to_string()
    };
    let parsed = url::Url::parse(raw).map_err(|_| invalid())?;
    if parsed.scheme() != "nxm"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port().is_some()
        || parsed.fragment().is_some()
    {
        return Err(invalid());
    }

    let domain = parsed.host_str().ok_or_else(invalid)?.to_ascii_lowercase();
    if domain.is_empty()
        || domain.len() > 100
        || !sanitize_url_path_segment(&domain).eq_ignore_ascii_case(&domain)
    {
        return Err(invalid());
    }

    let segments = parsed
        .path_segments()
        .ok_or_else(invalid)?
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.len() != 4 || segments[0] != "mods" || segments[2] != "files" {
        return Err(invalid());
    }
    let mod_id = segments[1].parse::<u64>().map_err(|_| invalid())?;
    let file_id = segments[3].parse::<u64>().map_err(|_| invalid())?;
    if mod_id == 0 || file_id == 0 {
        return Err(invalid());
    }

    let mut key = None;
    let mut expires = None;
    let mut user_id = None;
    for (name, value) in parsed.query_pairs() {
        match name.as_ref() {
            "key" => {
                if key.is_some() {
                    return Err(invalid());
                }
                key = Some(value.into_owned());
            }
            "expires" => {
                if expires.is_some() {
                    return Err(invalid());
                }
                expires = Some(value.into_owned());
            }
            "user_id" => {
                if user_id.is_some() {
                    return Err(invalid());
                }
                user_id = Some(value.into_owned());
            }
            _ => {}
        }
    }

    let key = key.ok_or_else(invalid)?;
    if key.is_empty() || key.len() > 1024 || key.chars().any(char::is_whitespace) {
        return Err(invalid());
    }
    let expires = expires
        .ok_or_else(invalid)?
        .parse::<i64>()
        .map_err(|_| invalid())?;
    if expires < Utc::now().timestamp() - 30 {
        return Err("This Nexus download authorization expired. Return to the file page and click Slow download again.".to_string());
    }
    let user_id = user_id
        .ok_or_else(invalid)?
        .parse::<u64>()
        .map_err(|_| invalid())?;
    if user_id == 0 {
        return Err(invalid());
    }

    Ok(NexusNxmLink {
        domain,
        mod_id,
        file_id,
        key,
        expires,
        user_id,
    })
}

fn pending_nexus_download_is_fresh(pending: &PendingNexusDownload, now: i64) -> bool {
    pending.created_at <= now + 60
        && now.saturating_sub(pending.created_at)
            <= NEXUS_PENDING_DOWNLOAD_TTL_MINUTES.saturating_mul(60)
}

fn store_pending_nexus_download(
    store_root: &Path,
    pending: PendingNexusDownload,
    now: i64,
) -> Result<(), String> {
    let path = pending_nexus_downloads_path(store_root);
    let mut store = read_store::<PendingNexusDownload>(&path).map_err(error_to_string)?;
    store.items.retain(|existing| {
        pending_nexus_download_is_fresh(existing, now)
            && !(existing.domain.eq_ignore_ascii_case(&pending.domain)
                && existing.mod_id == pending.mod_id
                && existing.file_id == pending.file_id)
    });
    store.items.push(pending);
    write_store(&path, &store).map_err(error_to_string)
}

fn find_pending_nexus_download(
    store_root: &Path,
    nxm: &NexusNxmLink,
) -> Result<PendingNexusDownload, String> {
    let path = pending_nexus_downloads_path(store_root);
    let mut store = read_store::<PendingNexusDownload>(&path).map_err(error_to_string)?;
    let profiles =
        read_store::<GameProfile>(&profiles_path(store_root)).map_err(error_to_string)?;
    let profile_ids = profiles
        .items
        .iter()
        .map(|profile| profile.id.as_str())
        .collect::<HashSet<_>>();
    let now = Utc::now().timestamp();
    let original_len = store.items.len();
    store.items.retain(|pending| {
        pending_nexus_download_is_fresh(pending, now)
            && profile_ids.contains(pending.profile_id.as_str())
    });
    if store.items.len() != original_len {
        write_store(&path, &store).map_err(error_to_string)?;
    }

    store
        .items
        .into_iter()
        .filter(|pending| {
            pending.domain.eq_ignore_ascii_case(&nxm.domain)
                && pending.mod_id == nxm.mod_id
                && pending.file_id == nxm.file_id
        })
        .max_by_key(|pending| pending.created_at)
        .ok_or_else(|| {
            "UniLoader did not request this Nexus download, or the request expired. Start it again from Discovery."
                .to_string()
        })
}

fn remove_pending_nexus_downloads_for_profile(
    store_root: &Path,
    profile_id: &str,
) -> Result<(), String> {
    let path = pending_nexus_downloads_path(store_root);
    let mut store = read_store::<PendingNexusDownload>(&path).map_err(error_to_string)?;
    let original_len = store.items.len();
    store
        .items
        .retain(|pending| pending.profile_id != profile_id);
    if store.items.len() != original_len {
        write_store(&path, &store).map_err(error_to_string)?;
    }
    Ok(())
}

fn remove_pending_nexus_download(store_root: &Path, nxm: &NexusNxmLink) -> Result<(), String> {
    let path = pending_nexus_downloads_path(store_root);
    let mut store = read_store::<PendingNexusDownload>(&path).map_err(error_to_string)?;
    store.items.retain(|pending| {
        !(pending.domain.eq_ignore_ascii_case(&nxm.domain)
            && pending.mod_id == nxm.mod_id
            && pending.file_id == nxm.file_id)
    });
    write_store(&path, &store).map_err(error_to_string)
}

fn fetch_nexus_mod_files(
    client: &Client,
    api_key: &str,
    domain: &str,
    mod_id: u64,
) -> Result<Vec<NexusFileRecord>, String> {
    let url = format!(
        "https://api.nexusmods.com/v1/games/{}/mods/{}/files.json",
        sanitize_url_path_segment(domain),
        mod_id
    );
    let response = nexus_api_get(client, &url, api_key)
        .send()
        .map_err(error_to_string)?
        .error_for_status()
        .map_err(error_to_string)?
        .json::<NexusFilesResponse>()
        .map_err(error_to_string)?;

    Ok(response.files)
}

fn fetch_nexus_download_links(
    client: &Client,
    api_key: &str,
    domain: &str,
    mod_id: u64,
    file_id: u64,
) -> Result<Vec<NexusDownloadLink>, String> {
    let url = format!(
        "https://api.nexusmods.com/v1/games/{}/mods/{}/files/{}/download_link.json",
        sanitize_url_path_segment(domain),
        mod_id,
        file_id
    );
    let response = nexus_api_get(client, &url, api_key)
        .send()
        .map_err(error_to_string)?;
    if response.status().as_u16() == 403 {
        return Err(
            "Nexus requires this download to be started on its website. Choose the file and use the browser handoff."
                .to_string(),
        );
    }
    response
        .error_for_status()
        .map_err(error_to_string)?
        .json::<Vec<NexusDownloadLink>>()
        .map_err(error_to_string)
}

fn fetch_nexus_download_links_with_nxm(
    client: &Client,
    api_key: &str,
    nxm: &NexusNxmLink,
) -> Result<Vec<NexusDownloadLink>, String> {
    let url = format!(
        "https://api.nexusmods.com/v1/games/{}/mods/{}/files/{}/download_link.json",
        sanitize_url_path_segment(&nxm.domain),
        nxm.mod_id,
        nxm.file_id
    );
    let expires = nxm.expires.to_string();
    let user_id = nxm.user_id.to_string();
    let response = nexus_api_get(client, &url, api_key)
        .query(&[
            ("key", nxm.key.as_str()),
            ("expires", expires.as_str()),
            ("user_id", user_id.as_str()),
        ])
        .send()
        .map_err(|_| "Could not contact Nexus to finish the authorized download.".to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(if status.as_u16() == 403 {
            "Nexus rejected this download authorization. Return to the file page and click Slow download again."
                .to_string()
        } else {
            format!(
                "Nexus could not finish the authorized download (HTTP {}). Try again.",
                status.as_u16()
            )
        });
    }

    response.json::<Vec<NexusDownloadLink>>().map_err(|_| {
        "Nexus returned invalid download-server data. Try the download again.".to_string()
    })
}

fn nexus_api_get(client: &Client, url: &str, api_key: &str) -> reqwest::blocking::RequestBuilder {
    client
        .get(url)
        .header("apikey", api_key)
        .header("Application-Name", "UniLoader")
        .header("Application-Version", env!("CARGO_PKG_VERSION"))
        .header("Accept", "application/json")
}

fn validate_nexus_api_key(api_key: &str) -> Result<NexusUserValidation, String> {
    let client = provider_client()?;
    let response = nexus_api_get(
        &client,
        "https://api.nexusmods.com/v1/users/validate.json",
        api_key,
    )
    .send()
    .map_err(|error| format!("Could not validate the Nexus API key: {error}"))?;

    if response.status().is_success() {
        return response.json::<NexusUserValidation>().map_err(|error| {
            format!("Nexus validated the key but returned invalid account data: {error}")
        });
    }
    if response.status().as_u16() == 401 || response.status().as_u16() == 403 {
        return Err(
            "Nexus Mods rejected this API key. Request a personal API key and paste the full value."
                .to_string(),
        );
    }

    Err(format!(
        "Nexus Mods could not validate the API key right now (HTTP {}). Please try again.",
        response.status()
    ))
}

fn choose_requested_nexus_file(
    files: &[NexusFileRecord],
    selected_file_id: Option<&str>,
) -> Result<Option<NexusFileRecord>, String> {
    let Some(selected_file_id) = selected_file_id else {
        return Ok(choose_nexus_file(files));
    };
    let parsed_file_id = selected_file_id
        .parse::<u64>()
        .map_err(|_| "The selected Nexus file id is invalid.".to_string())?;
    let file = files
        .iter()
        .find(|file| file.file_id == parsed_file_id)
        .cloned()
        .ok_or_else(|| "The selected Nexus file is no longer available.".to_string())?;
    if !nexus_file_supported(&file) {
        return Err(
            "The selected Nexus file is not a ZIP, 7Z, or RAR archive that UniLoader can install."
                .to_string(),
        );
    }
    Ok(Some(file))
}

fn choose_nexus_file(files: &[NexusFileRecord]) -> Option<NexusFileRecord> {
    files
        .iter()
        .filter(|file| nexus_file_supported(file))
        .max_by_key(|file| {
            let category_score = nexus_file_category_score(file);
            let primary_score = if file.is_primary.unwrap_or(false) {
                1
            } else {
                0
            };
            let upload_time = file.uploaded_timestamp.unwrap_or(0);
            (category_score, primary_score, upload_time)
        })
        .cloned()
}

fn nexus_file_supported(file: &NexusFileRecord) -> bool {
    file.file_name
        .as_deref()
        .map(|name| {
            let lower = name.to_lowercase();
            lower.ends_with(".zip") || lower.ends_with(".7z") || lower.ends_with(".rar")
        })
        .unwrap_or(true)
}

fn nexus_file_category_score(file: &NexusFileRecord) -> u8 {
    let category = file
        .category_name
        .as_deref()
        .unwrap_or_default()
        .to_lowercase();
    if file.is_primary.unwrap_or(false) || category.contains("main") {
        3
    } else if category.contains("update") {
        2
    } else if category.contains("optional") {
        1
    } else if category.contains("old") {
        0
    } else {
        2
    }
}

fn nexus_http_download_url(links: &[NexusDownloadLink]) -> Option<String> {
    links.iter().find_map(|link| {
        let uri = link.uri.as_deref()?.trim();
        if uri.starts_with("http://") || uri.starts_with("https://") {
            Some(uri.to_string())
        } else {
            None
        }
    })
}

fn download_nexus_file(
    store_root: &Path,
    client: &Client,
    domain: &str,
    mod_id: u64,
    file: &NexusFileRecord,
    download_url: &str,
) -> Result<PathBuf, String> {
    let file_name = file
        .file_name
        .as_deref()
        .or_else(|| {
            let uri_name = download_url
                .split('/')
                .next_back()
                .and_then(|name| name.split('?').next());
            uri_name.filter(|name| !name.trim().is_empty())
        })
        .unwrap_or("nexus-mod.zip");
    let safe_name = sanitize_file_segment(file_name);
    let download_dir = store_root.join("downloads").join("nexus").join(format!(
        "{}-{}-{}",
        sanitize_file_segment(domain),
        mod_id,
        file.file_id
    ));
    fs::create_dir_all(&download_dir).map_err(error_to_string)?;
    let archive_path = download_dir.join(format!("{}-{}", Uuid::new_v4(), safe_name));
    download_url_to_file(client, download_url, &archive_path)?;
    Ok(archive_path)
}

fn nexus_file_display_name(file: &NexusFileRecord) -> String {
    non_empty_string(file.name.trim())
        .or_else(|| {
            file.file_name
                .as_deref()
                .and_then(|name| non_empty_string(name.trim()))
        })
        .unwrap_or_else(|| format!("Nexus file {}", file.file_id))
}

fn install_thunderstore_discovered_mod(
    store_root: &Path,
    profile: &GameProfile,
    mod_id: &str,
    version: Option<String>,
    supplied_provider_game_id: Option<&str>,
) -> Result<InstallResult, String> {
    let raw_id = mod_id
        .strip_prefix("thunderstore:")
        .ok_or_else(|| format!("Invalid Thunderstore mod id: {}", mod_id))?;
    let package_ref = parse_thunderstore_token(raw_id, version.as_deref()).ok_or_else(|| {
        format!(
            "Could not parse Thunderstore package reference from {}.",
            mod_id
        )
    })?;
    let provider_game_id = verified_discovery_provider_game(
        profile,
        "thunderstore",
        supplied_provider_game_id,
        supplied_provider_game_id,
    )?;
    let package_version = fetch_thunderstore_package_version(&package_ref)?;
    let resolved_ref = ThunderstorePackageRef {
        namespace: package_ref.namespace.clone(),
        name: package_ref.name.clone(),
        version: Some(package_version.version_number.clone()),
    };
    let package_id = thunderstore_package_id(&package_ref);
    let dependency_string =
        thunderstore_dependency_string(&package_ref, &package_version.version_number);
    let provider_document = thunderstore_route_document(&resolved_ref, &package_version);
    let provided_runtime = runtime_id_for_provider_package(
        profile,
        "thunderstore",
        Some(&package_ref.namespace),
        &package_ref.name,
    );

    if thunderstore_package_installed(store_root, profile, &package_id, Some(&dependency_string)) {
        return Err(format!(
            "{} is already installed in this profile.",
            humanize_mod_display_name(&package_version.full_name)
        ));
    }

    let archive_path = download_thunderstore_package(store_root, &resolved_ref, &package_version)?;
    let source_identity = provider_source_identity(
        "thunderstore",
        package_id.clone(),
        Some(package_version.version_number.clone()),
        Some(provider_game_id),
        "Thunderstore community catalogue and package manifest",
    );
    let scanned = scan_zip_archive(&archive_path)?;
    let route_outcome =
        prepare_package_install_routes(store_root, profile, &scanned, &provider_document);
    let analysis = analyze_scanned_archive_with_identity(scanned, profile, Some(source_identity));
    let package_identity = analysis.package_identity.clone();
    let mut plan = analysis
        .recommended_plan
        .ok_or_else(|| analysis.compatibility.reason.clone())?;

    for dependency_string in &package_version.dependencies {
        let parsed = parse_thunderstore_dependency(dependency_string);
        if !plan.dependencies.iter().any(|item| item.id == parsed.id) {
            plan.dependencies.push(parsed);
        }
    }
    append_unique_dependencies(
        &mut plan.dependencies,
        description_runtime_dependencies(
            profile,
            &provider_document.text,
            provided_runtime.as_deref(),
        ),
    );
    plan.warnings.extend(route_outcome.warnings);

    if plan.adapter_id == "loose-files" || plan.requires_confirmation {
        return Err(format!(
            "{} downloaded, but UniLoader could not identify a safe automatic install layout.",
            package_version.full_name
        ));
    }

    let archive_path_string = archive_path.to_string_lossy().to_string();
    let mut visited_dependencies = HashSet::new();
    install_archive_impl_with_metadata(
        store_root,
        profile,
        &archive_path_string,
        &plan,
        InstallOptions {
            metadata: InstallMetadata {
                archive_name: Some(package_version.full_name.clone()),
                package_id: Some(package_id),
                dependency_string: Some(dependency_string),
                display_name: Some(package_version.full_name),
                package_identity: Some(package_identity),
                runtime_id: None,
                icon_url: package_version.icon.clone(),
            },
            resolve_dependencies: true,
            visited_dependencies: &mut visited_dependencies,
            dependency_depth: 0,
        },
    )
}

fn install_thunderstore_dependency(
    store_root: &Path,
    profile: &GameProfile,
    dependency: &DependencySpec,
    visited_dependencies: &mut HashSet<String>,
    dependency_depth: usize,
) -> Result<Vec<String>, String> {
    if dependency_depth > MAX_DEPENDENCY_DEPTH {
        return Err("Dependency chain is too deep to install safely.".to_string());
    }

    let package_ref = parse_thunderstore_ref(dependency)?;
    let visit_key = thunderstore_visit_key(&package_ref);
    if !visited_dependencies.insert(visit_key) {
        return Ok(Vec::new());
    }

    if dependency_already_available(store_root, profile, dependency) {
        return Ok(Vec::new());
    }

    let package_version = fetch_thunderstore_package_version(&package_ref)?;
    let resolved_ref = ThunderstorePackageRef {
        namespace: package_ref.namespace.clone(),
        name: package_ref.name.clone(),
        version: Some(package_version.version_number.clone()),
    };
    visited_dependencies.insert(thunderstore_visit_key(&resolved_ref));
    let package_id = thunderstore_package_id(&package_ref);
    let dependency_string =
        thunderstore_dependency_string(&package_ref, &package_version.version_number);
    let provider_document = thunderstore_route_document(&resolved_ref, &package_version);
    let provided_runtime = runtime_id_for_provider_package(
        profile,
        "thunderstore",
        Some(&package_ref.namespace),
        &package_ref.name,
    );

    if thunderstore_package_installed(store_root, profile, &package_id, Some(&dependency_string))
        || thunderstore_runtime_available(profile, &package_ref)
    {
        return Ok(Vec::new());
    }

    let archive_path = download_thunderstore_package(store_root, &resolved_ref, &package_version)?;
    let scanned = scan_zip_archive(&archive_path)?;
    let route_outcome =
        prepare_package_install_routes(store_root, profile, &scanned, &provider_document);
    let analysis = analyze_scanned_archive(scanned, profile);
    let mut plan = analysis.recommended_plan.ok_or_else(|| {
        format!(
            "Could not find a safe install route for dependency {}.",
            dependency.name
        )
    })?;

    for dependency_string in &package_version.dependencies {
        let parsed = parse_thunderstore_dependency(dependency_string);
        if !plan.dependencies.iter().any(|item| item.id == parsed.id) {
            plan.dependencies.push(parsed);
        }
    }
    append_unique_dependencies(
        &mut plan.dependencies,
        description_runtime_dependencies(
            profile,
            &provider_document.text,
            provided_runtime.as_deref(),
        ),
    );
    plan.warnings.extend(route_outcome.warnings);

    if plan.adapter_id == "loose-files" || plan.requires_confirmation {
        return Err(format!(
            "Dependency {} downloaded, but UniLoader could not identify a safe automatic install layout.",
            dependency.name
        ));
    }

    let archive_path_string = archive_path.to_string_lossy().to_string();
    let result = install_archive_impl_with_metadata(
        store_root,
        profile,
        &archive_path_string,
        &plan,
        InstallOptions {
            metadata: InstallMetadata {
                archive_name: Some(package_version.full_name.clone()),
                package_id: Some(package_id),
                dependency_string: Some(dependency_string),
                display_name: Some(package_version.full_name.clone()),
                package_identity: None,
                runtime_id: runtime_id_for_dependency(profile, dependency),
                icon_url: package_version.icon.clone(),
            },
            resolve_dependencies: true,
            visited_dependencies,
            dependency_depth,
        },
    )?;

    let mut warnings = result.warnings;
    warnings.push(format!(
        "Installed dependency {} {}.",
        package_version.full_name, package_version.version_number
    ));
    Ok(warnings)
}

fn install_release_dependency(
    store_root: &Path,
    profile: &GameProfile,
    dependency: &DependencySpec,
    release_ref: ReleaseDependencyRef,
    visited_dependencies: &mut HashSet<String>,
    dependency_depth: usize,
) -> Result<Vec<String>, String> {
    if dependency_depth > MAX_DEPENDENCY_DEPTH {
        return Err("Dependency chain is too deep to install safely.".to_string());
    }

    let visit_key = format!("release:{}", release_ref.source_key.to_lowercase());
    if !visited_dependencies.insert(visit_key) {
        return Ok(Vec::new());
    }

    if dependency_already_available(store_root, profile, dependency) {
        return Ok(Vec::new());
    }

    let archive_path = download_release_archive(store_root, dependency, &release_ref)?;
    let scanned = scan_zip_archive(&archive_path)?;
    let analysis = analyze_scanned_archive(scanned, profile);
    let plan = analysis.recommended_plan.ok_or_else(|| {
        format!(
            "Could not find a safe install route for dependency {}.",
            dependency.name
        )
    })?;

    if plan.adapter_id == "loose-files" || plan.requires_confirmation {
        return Err(format!(
            "Dependency {} downloaded, but UniLoader could not identify a safe automatic install layout.",
            dependency.name
        ));
    }

    let archive_path_string = archive_path.to_string_lossy().to_string();
    let result = install_archive_impl_with_metadata(
        store_root,
        profile,
        &archive_path_string,
        &plan,
        InstallOptions {
            metadata: InstallMetadata {
                archive_name: Some(release_ref.display_name.clone()),
                package_id: Some(release_ref.source_key.clone()),
                dependency_string: Some(format!(
                    "{}@{}",
                    release_ref.source_key, release_ref.version
                )),
                display_name: Some(release_ref.display_name.clone()),
                package_identity: None,
                runtime_id: runtime_id_for_dependency(profile, dependency),
                icon_url: None,
            },
            resolve_dependencies: true,
            visited_dependencies,
            dependency_depth,
        },
    )?;

    let mut warnings = result.warnings;
    warnings.push(format!(
        "Installed dependency {} {}.",
        release_ref.display_name, release_ref.version
    ));
    Ok(warnings)
}

fn refresh_dependency_status(
    store_root: &Path,
    profile: &GameProfile,
    dependency: &DependencySpec,
) -> DependencySpec {
    let mut refreshed = dependency.clone();
    if dependency_already_available(store_root, profile, dependency) {
        refreshed.status = "already-installed".to_string();
    }
    refreshed
}

fn dependency_already_available(
    store_root: &Path,
    profile: &GameProfile,
    dependency: &DependencySpec,
) -> bool {
    if let Some(runtime) = runtime_from_dependency(dependency) {
        if runtime_installed(profile, runtime) {
            return true;
        }
    }

    if dependency.provider == "nexus" {
        let Ok(store) = read_store::<InstalledModRecord>(&installed_mods_path(store_root)) else {
            return false;
        };
        return store.items.iter().any(|record| {
            record.profile_id == profile.id
                && record.enabled
                && record.last_status != "removed"
                && record
                    .package_id
                    .as_deref()
                    .is_some_and(|package_id| package_id.eq_ignore_ascii_case(&dependency.id))
        });
    }

    if dependency.provider != "thunderstore" {
        return dependency.status == "already-installed";
    }

    let Ok(package_ref) = parse_thunderstore_ref(dependency) else {
        return false;
    };

    if thunderstore_runtime_available(profile, &package_ref) {
        return true;
    }

    let package_id = thunderstore_package_id(&package_ref);
    let dependency_string = package_ref
        .version
        .as_ref()
        .map(|version| thunderstore_dependency_string(&package_ref, version));

    thunderstore_package_installed(
        store_root,
        profile,
        &package_id,
        dependency_string.as_deref(),
    )
}

fn push_missing_dependency(
    missing_dependencies: &mut Vec<DependencySpec>,
    seen: &mut HashSet<String>,
    dependency: DependencySpec,
) {
    if !dependency.required || dependency.status == "already-installed" {
        return;
    }

    if seen.insert(dependency_key(&dependency)) {
        missing_dependencies.push(dependency);
    }
}

fn dependency_key(dependency: &DependencySpec) -> String {
    format!(
        "{}|{}|{}",
        dependency.provider,
        dependency.id,
        dependency.version.as_deref().unwrap_or_default()
    )
}

fn mod_file_health_for_record(
    record: &InstalledModRecord,
    launch_suspension: &ProfileLaunchSuspension,
) -> ModFileHealth {
    let checked_files = if record.enabled {
        record.files_written.len()
    } else {
        0
    };
    let intentionally_suspended = record.enabled
        && launch_suspension
            .mods
            .iter()
            .any(|suspended| suspended.mod_id == record.id);
    let mut missing_files = Vec::new();
    let mut suspended_files = Vec::new();

    if record.enabled {
        for path in &record.files_written {
            if Path::new(path).exists() {
                continue;
            }
            if intentionally_suspended {
                suspended_files.push(path.clone());
            } else {
                missing_files.push(path.clone());
            }
        }
    }

    ModFileHealth {
        installed_mod_id: record.id.clone(),
        mod_name: record
            .display_name
            .as_deref()
            .map(humanize_mod_display_name)
            .unwrap_or_else(|| humanize_mod_display_name(&record.archive_name)),
        checked_files,
        missing_files,
        suspended_files,
    }
}

fn profile_refresh_warnings(
    detection: &GameDetectionResult,
    mod_file_health: &[ModFileHealth],
    missing_dependencies: &[DependencySpec],
) -> Vec<String> {
    let mut warnings = detection.warnings.clone();
    let missing_file_count = mod_file_health
        .iter()
        .map(|health| health.missing_files.len())
        .sum::<usize>();

    if missing_file_count > 0 {
        let affected_mod_count = mod_file_health
            .iter()
            .filter(|health| !health.missing_files.is_empty())
            .count();
        warnings.push(format!(
            "{} expected file(s) are missing across {} enabled mod(s).",
            missing_file_count, affected_mod_count
        ));
    }

    if !missing_dependencies.is_empty() {
        warnings.push(format!(
            "{} required dependenc{} still missing.",
            missing_dependencies.len(),
            if missing_dependencies.len() == 1 {
                "y is"
            } else {
                "ies are"
            }
        ));
    }

    warnings
}

fn runtime_from_dependency(dependency: &DependencySpec) -> Option<&str> {
    let (kind, runtime) = dependency.id.split_once(':')?;
    (kind.eq_ignore_ascii_case("runtime") && !runtime.trim().is_empty()).then_some(runtime)
}

fn runtime_installed(profile: &GameProfile, runtime: &str) -> bool {
    let game_path = Path::new(&profile.game_path);
    if !game_path.is_dir() {
        return false;
    }

    let Some(definition) = runtime_definition_by_id(runtime) else {
        return false;
    };
    let entries = walk_game_folder(game_path);
    definition
        .detection_rules
        .iter()
        .any(|rule| runtime_detection_rule_matches(profile, &entries, rule))
}

fn runtime_supplied_by_plan(profile: &GameProfile, plan: &InstallPlan) -> Option<String> {
    let mut entries = Vec::new();
    let mut seen_entries = HashSet::new();

    for mapping in plan
        .mappings
        .iter()
        .filter(|mapping| mapping.target_root.eq_ignore_ascii_case("game"))
    {
        let relative_path = normalize_archive_path(&mapping.target_relative_path);
        let segments = relative_path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();
        if segments.is_empty() {
            continue;
        }

        for depth in 1..segments.len() {
            let directory = segments[..depth].join("/");
            if seen_entries.insert((directory.to_ascii_lowercase(), true)) {
                entries.push(ProbeEntry {
                    relative_path: directory.clone(),
                    name: basename(&directory),
                    is_directory: true,
                    depth,
                });
            }
        }

        if seen_entries.insert((relative_path.to_ascii_lowercase(), false)) {
            entries.push(ProbeEntry {
                relative_path: relative_path.clone(),
                name: basename(&relative_path),
                is_directory: false,
                depth: segments.len(),
            });
        }
    }

    profile_runtime_ids(profile).into_iter().find(|runtime| {
        runtime_definition_by_id(runtime).is_some_and(|definition| {
            definition
                .detection_rules
                .iter()
                .any(|rule| runtime_detection_rule_matches(profile, &entries, rule))
        })
    })
}

fn runtime_detection_rule_matches(
    profile: &GameProfile,
    entries: &[ProbeEntry],
    rule: &RuntimeDetectionRule,
) -> bool {
    if rule.all.is_empty() && rule.any.is_empty() {
        return false;
    }

    rule.all
        .iter()
        .all(|marker| runtime_detection_marker_matches(profile, entries, marker))
        && (rule.any.is_empty()
            || rule
                .any
                .iter()
                .any(|marker| runtime_detection_marker_matches(profile, entries, marker)))
}

fn runtime_detection_marker_matches(
    profile: &GameProfile,
    entries: &[ProbeEntry],
    marker: &str,
) -> bool {
    let Some((kind, expected)) = marker.split_once(':') else {
        return false;
    };
    let expected = normalize_archive_path(expected.trim());

    match kind.trim().to_ascii_lowercase().as_str() {
        "profile-engine" => profile.engine.eq_ignore_ascii_case(&expected),
        "profile-loader" => profile.loader.eq_ignore_ascii_case(&expected),
        "file" => entries.iter().any(|entry| {
            !entry.is_directory
                && normalize_archive_path(&entry.relative_path).eq_ignore_ascii_case(&expected)
        }),
        "file-name" => entries
            .iter()
            .any(|entry| !entry.is_directory && entry.name.eq_ignore_ascii_case(&expected)),
        "dir" => entries.iter().any(|entry| {
            entry.is_directory
                && normalize_archive_path(&entry.relative_path).eq_ignore_ascii_case(&expected)
        }),
        "dir-prefix" => entries.iter().any(|entry| {
            entry.is_directory && path_matches_or_is_below(&entry.relative_path, expected.as_str())
        }),
        "path" => entries.iter().any(|entry| {
            normalize_archive_path(&entry.relative_path).eq_ignore_ascii_case(&expected)
        }),
        "path-prefix" => entries
            .iter()
            .any(|entry| path_matches_or_is_below(&entry.relative_path, expected.as_str())),
        _ => false,
    }
}

fn path_matches_or_is_below(path: &str, expected: &str) -> bool {
    let normalized_path = normalize_archive_path(path).to_ascii_lowercase();
    let normalized_expected = normalize_archive_path(expected).to_ascii_lowercase();
    normalized_path == normalized_expected
        || normalized_path.starts_with(&format!("{normalized_expected}/"))
}

fn thunderstore_runtime_available(
    profile: &GameProfile,
    package_ref: &ThunderstorePackageRef,
) -> bool {
    runtime_definitions().iter().any(|definition| {
        runtime_definition_applies_to_profile(definition, profile)
            && runtime_provider_package_matches(
                definition,
                "thunderstore",
                Some(&package_ref.namespace),
                &package_ref.name,
            )
            && runtime_installed(profile, &definition.id)
    })
}

fn runtime_definition_applies_to_profile(
    definition: &RuntimeDefinition,
    profile: &GameProfile,
) -> bool {
    if definition.profile_engines.is_empty() && definition.profile_loaders.is_empty() {
        return true;
    }

    definition
        .profile_engines
        .iter()
        .any(|engine| engine.eq_ignore_ascii_case(&profile.engine))
        || definition
            .profile_loaders
            .iter()
            .any(|loader| loader.eq_ignore_ascii_case(&profile.loader))
}

fn runtime_provider_package_matches(
    definition: &RuntimeDefinition,
    provider: &str,
    namespace: Option<&str>,
    package_name: &str,
) -> bool {
    definition.provider_packages.iter().any(|package| {
        package.provider.eq_ignore_ascii_case(provider)
            && (package.namespace_patterns.is_empty()
                || namespace.is_some_and(|value| {
                    package
                        .namespace_patterns
                        .iter()
                        .any(|pattern| wildcard_match(pattern, value))
                }))
            && package.package_patterns.iter().any(|pattern| {
                wildcard_match(pattern, package_name)
                    || game_qualified_runtime_name_matches(pattern, package_name)
            })
    })
}

fn game_qualified_runtime_name_matches(runtime_alias: &str, package_name: &str) -> bool {
    if runtime_alias.contains('*') || runtime_alias.contains('?') {
        return false;
    }

    let alias = normalized_runtime_package_name(runtime_alias);
    let package = normalized_runtime_package_name(package_name);
    if alias.is_empty() || package.len() <= alias.len() || !package.starts_with(&alias) {
        return false;
    }

    let suffix = package[alias.len()..].trim_start();
    suffix
        .strip_prefix("for ")
        .is_some_and(|game_name| !game_name.trim().is_empty())
}

fn normalized_runtime_package_name(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn thunderstore_package_installed(
    store_root: &Path,
    profile: &GameProfile,
    package_id: &str,
    dependency_string: Option<&str>,
) -> bool {
    let Ok(store) = read_store::<InstalledModRecord>(&installed_mods_path(store_root)) else {
        return false;
    };

    store.items.iter().any(|record| {
        record.profile_id == profile.id
            && record.enabled
            && (record
                .package_id
                .as_deref()
                .map(|installed_id| installed_id.eq_ignore_ascii_case(package_id))
                .unwrap_or(false)
                || dependency_string
                    .and_then(|expected| {
                        record
                            .dependency_string
                            .as_deref()
                            .map(|installed| installed.eq_ignore_ascii_case(expected))
                    })
                    .unwrap_or(false))
    })
}

fn parse_thunderstore_ref(dependency: &DependencySpec) -> Result<ThunderstorePackageRef, String> {
    if dependency.provider != "thunderstore" {
        return Err(format!(
            "{} is not a Thunderstore dependency.",
            dependency.name
        ));
    }

    if let Some((namespace, name)) = dependency.name.split_once('/') {
        return Ok(ThunderstorePackageRef {
            namespace: namespace.to_string(),
            name: name.to_string(),
            version: dependency.version.clone(),
        });
    }

    let raw = dependency
        .id
        .strip_prefix("thunderstore:")
        .unwrap_or(&dependency.id);
    parse_thunderstore_token(raw, dependency.version.as_deref()).ok_or_else(|| {
        format!(
            "Could not parse Thunderstore dependency: {}",
            dependency.name
        )
    })
}

fn parse_thunderstore_token(
    token: &str,
    version_hint: Option<&str>,
) -> Option<ThunderstorePackageRef> {
    let token = token
        .trim()
        .strip_prefix("thunderstore:")
        .unwrap_or(token.trim());
    if let Some((namespace, name)) = token.split_once('/') {
        return Some(ThunderstorePackageRef {
            namespace: namespace.to_string(),
            name: name.to_string(),
            version: version_hint.map(|value| value.to_string()),
        });
    }

    let mut package_token = token.to_string();
    let mut version = version_hint.map(|value| value.to_string());
    if version.is_none() {
        if let Some((without_version, possible_version)) = token.rsplit_once('-') {
            if looks_like_version(possible_version) {
                package_token = without_version.to_string();
                version = Some(possible_version.to_string());
            }
        }
    } else if let Some(version_value) = &version {
        let suffix = format!("-{}", version_value);
        if package_token.ends_with(&suffix) {
            package_token.truncate(package_token.len() - suffix.len());
        }
    }

    let parts = package_token.split('-').collect::<Vec<_>>();
    if parts.len() < 2 {
        return None;
    }

    let (namespace, name) = if version.is_some() {
        (
            parts[..parts.len() - 1].join("-"),
            parts[parts.len() - 1].to_string(),
        )
    } else {
        let (namespace, name) = package_token.split_once('-')?;
        (namespace.to_string(), name.to_string())
    };

    if namespace.is_empty() || name.is_empty() {
        None
    } else {
        Some(ThunderstorePackageRef {
            namespace,
            name,
            version,
        })
    }
}

fn looks_like_version(value: &str) -> bool {
    value
        .chars()
        .next()
        .map(|character| character.is_ascii_digit())
        .unwrap_or(false)
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        })
}

fn fetch_thunderstore_package_version(
    package_ref: &ThunderstorePackageRef,
) -> Result<ThunderstoreVersion, String> {
    let client = thunderstore_client()?;
    if let Some(requested_version) = &package_ref.version {
        let version_url = thunderstore_package_version_url(package_ref, requested_version);
        return client
            .get(version_url)
            .send()
            .map_err(error_to_string)?
            .error_for_status()
            .map_err(|error| {
                format!(
                    "Thunderstore package {}/{} does not have version {}: {}",
                    package_ref.namespace, package_ref.name, requested_version, error
                )
            })?
            .json::<ThunderstoreVersion>()
            .map_err(error_to_string);
    }

    let url = format!(
        "{}/{}/{}",
        THUNDERSTORE_API_BASE, package_ref.namespace, package_ref.name
    );
    client
        .get(url)
        .send()
        .map_err(error_to_string)?
        .error_for_status()
        .map_err(error_to_string)?
        .json::<ThunderstorePackageResponse>()
        .map(|response| response.latest)
        .map_err(error_to_string)
}

fn thunderstore_package_version_url(package_ref: &ThunderstorePackageRef, version: &str) -> String {
    format!(
        "{}/{}/{}/{}",
        THUNDERSTORE_API_BASE, package_ref.namespace, package_ref.name, version
    )
}

fn download_thunderstore_package(
    store_root: &Path,
    package_ref: &ThunderstorePackageRef,
    package_version: &ThunderstoreVersion,
) -> Result<PathBuf, String> {
    let cache_dir = store_root
        .join("cache")
        .join("thunderstore")
        .join(sanitize_file_segment(&package_ref.namespace))
        .join(sanitize_file_segment(&package_ref.name));
    fs::create_dir_all(&cache_dir).map_err(error_to_string)?;

    let archive_path = cache_dir.join(format!(
        "{}-{}.zip",
        sanitize_file_segment(&package_version.full_name),
        sanitize_file_segment(&package_version.version_number)
    ));

    if archive_path.exists()
        && archive_path
            .metadata()
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false)
    {
        return Ok(archive_path);
    }

    let client = thunderstore_client()?;
    download_url_to_file(&client, &package_version.download_url, &archive_path)?;
    Ok(archive_path)
}

fn resolve_github_release_dependency(
    dependency: &DependencySpec,
) -> Result<ReleaseDependencyRef, String> {
    let source = dependency
        .source
        .as_deref()
        .ok_or_else(|| format!("{} is missing a GitHub release source.", dependency.name))?;
    let raw = source
        .strip_prefix("github-release:")
        .ok_or_else(|| format!("{} has an invalid GitHub release source.", dependency.name))?;
    let (repo, asset_pattern) = raw.split_once('#').ok_or_else(|| {
        format!(
            "{} GitHub source must include an asset pattern.",
            dependency.name
        )
    })?;
    let (owner, repo_name) = repo
        .split_once('/')
        .ok_or_else(|| format!("{} GitHub source must use owner/repo.", dependency.name))?;

    let client = provider_client()?;
    let url = format!(
        "{}/{}/{}/releases/latest",
        GITHUB_API_BASE, owner, repo_name
    );
    let release = client
        .get(url)
        .send()
        .map_err(error_to_string)?
        .error_for_status()
        .map_err(error_to_string)?
        .json::<GithubReleaseResponse>()
        .map_err(error_to_string)?;

    let asset = release
        .assets
        .into_iter()
        .find(|asset| wildcard_match(asset_pattern, &asset.name))
        .ok_or_else(|| {
            format!(
                "Latest GitHub release for {} does not include asset pattern {}.",
                repo, asset_pattern
            )
        })?;

    Ok(ReleaseDependencyRef {
        source_key: source.to_string(),
        display_name: format!("{} {}", dependency.name, asset.name),
        download_url: asset.browser_download_url,
        version: release.tag_name,
    })
}

fn resolve_bepinbuilds_dependency(
    dependency: &DependencySpec,
) -> Result<ReleaseDependencyRef, String> {
    let source = dependency
        .source
        .as_deref()
        .ok_or_else(|| format!("{} is missing a BepInBuilds source.", dependency.name))?;
    let raw = source
        .strip_prefix("bepinbuilds:")
        .ok_or_else(|| format!("{} has an invalid BepInBuilds source.", dependency.name))?;
    let (project, asset_pattern) = raw.split_once('#').ok_or_else(|| {
        format!(
            "{} BepInBuilds source must include an asset pattern.",
            dependency.name
        )
    })?;

    let project_url = match project {
        "bepinex_be" => BEPINBUILDS_BEPINEX_BE,
        _ => return Err(format!("Unsupported BepInBuilds project: {}", project)),
    };

    let client = provider_client()?;
    let html = client
        .get(project_url)
        .send()
        .map_err(error_to_string)?
        .error_for_status()
        .map_err(error_to_string)?
        .text()
        .map_err(error_to_string)?;

    for href in extract_href_values(&html) {
        if !href.to_lowercase().ends_with(".zip") {
            continue;
        }

        let file_name = href
            .rsplit('/')
            .next()
            .map(minimal_url_decode)
            .unwrap_or_else(|| href.clone());

        if !wildcard_match(asset_pattern, &file_name) {
            continue;
        }

        let download_url = if href.starts_with("http://") || href.starts_with("https://") {
            href
        } else {
            format!("{}{}", BEPINBUILDS_BASE, href)
        };

        return Ok(ReleaseDependencyRef {
            source_key: source.to_string(),
            display_name: format!("{} {}", dependency.name, file_name),
            download_url,
            version: file_name
                .strip_suffix(".zip")
                .unwrap_or(&file_name)
                .to_string(),
        });
    }

    Err(format!(
        "BepInBuilds project {} does not include asset pattern {}.",
        project, asset_pattern
    ))
}

fn download_release_archive(
    store_root: &Path,
    dependency: &DependencySpec,
    release_ref: &ReleaseDependencyRef,
) -> Result<PathBuf, String> {
    let cache_dir = store_root
        .join("cache")
        .join(sanitize_file_segment(&dependency.provider))
        .join(sanitize_file_segment(&release_ref.source_key))
        .join(sanitize_file_segment(&release_ref.version));
    fs::create_dir_all(&cache_dir).map_err(error_to_string)?;

    let file_name = release_ref
        .download_url
        .split('?')
        .next()
        .and_then(|path| path.rsplit('/').next())
        .map(minimal_url_decode)
        .filter(|name| name.to_lowercase().ends_with(".zip"))
        .unwrap_or_else(|| format!("{}.zip", sanitize_file_segment(&dependency.name)));
    let archive_path = cache_dir.join(sanitize_file_segment(&file_name));

    if archive_path.exists()
        && archive_path
            .metadata()
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false)
    {
        return Ok(archive_path);
    }

    let client = provider_client()?;
    download_url_to_file(&client, &release_ref.download_url, &archive_path)?;
    Ok(archive_path)
}

fn download_url_to_file(client: &Client, url: &str, destination: &Path) -> Result<(), String> {
    validate_https_url(url)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(error_to_string)?;
    }

    let temp_path = destination.with_extension("download");
    if temp_path.exists() {
        fs::remove_file(&temp_path).map_err(error_to_string)?;
    }

    let mut response = request_download_with_retry(client, url)?;

    let archive_extension = destination
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .filter(|extension| matches!(extension.as_str(), "zip" | "7z" | "rar"));
    if archive_extension.is_some() {
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if content_type.starts_with("text/")
            || content_type.contains("application/json")
            || content_type.contains("application/problem+json")
        {
            return Err(
                "The download provider returned a webpage or message instead of a mod archive. Start the download again."
                    .to_string(),
            );
        }
    }

    if let Some(content_length) = response.content_length() {
        if content_length > MAX_DOWNLOAD_BYTES {
            return Err(format!(
                "Download is too large: {} MB exceeds UniLoader's {} MB safety limit.",
                content_length / 1024 / 1024,
                MAX_DOWNLOAD_BYTES / 1024 / 1024
            ));
        }
    }

    let mut file = File::create(&temp_path).map_err(error_to_string)?;
    let bytes_written = match copy_with_limit(&mut response, &mut file, MAX_DOWNLOAD_BYTES) {
        Ok(bytes) => bytes,
        Err(error) => {
            let _ = fs::remove_file(&temp_path);
            return Err(error);
        }
    };
    file.sync_all().map_err(error_to_string)?;
    drop(file);

    if bytes_written == 0 {
        let _ = fs::remove_file(&temp_path);
        return Err("Download completed, but the downloaded file was empty.".to_string());
    }

    if let Some(extension) = archive_extension.as_deref() {
        if let Err(error) = validate_downloaded_archive(&temp_path, extension) {
            let _ = fs::remove_file(&temp_path);
            return Err(error);
        }
    }

    let result = replace_file_from_path(&temp_path, destination);
    let _ = fs::remove_file(&temp_path);
    result
}

fn request_download_with_retry(client: &Client, url: &str) -> Result<Response, String> {
    for attempt in 0..MAX_DOWNLOAD_ATTEMPTS {
        match client.get(url).send() {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    return Ok(response);
                }

                let should_retry =
                    is_transient_download_status(status) && attempt + 1 < MAX_DOWNLOAD_ATTEMPTS;
                if should_retry {
                    let delay = download_retry_delay(&response, attempt);
                    drop(response);
                    std::thread::sleep(delay);
                    continue;
                }

                return Err(format!(
                    "Download server returned HTTP {}.",
                    status.as_u16()
                ));
            }
            Err(error) => {
                let should_retry = (error.is_connect() || error.is_timeout())
                    && attempt + 1 < MAX_DOWNLOAD_ATTEMPTS;
                if should_retry {
                    std::thread::sleep(default_download_retry_delay(attempt));
                    continue;
                }

                return Err(format!("Download request failed: {}", error.without_url()));
            }
        }
    }

    Err("The download server did not respond after several attempts.".to_string())
}

fn is_transient_download_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn download_retry_delay(response: &Response, attempt: usize) -> Duration {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|seconds| Duration::from_secs(seconds.min(DOWNLOAD_RETRY_MAX_DELAY_SECS)))
        .unwrap_or_else(|| default_download_retry_delay(attempt))
}

fn default_download_retry_delay(attempt: usize) -> Duration {
    Duration::from_millis(DOWNLOAD_RETRY_BASE_DELAY_MS * (attempt as u64 + 1))
}

fn validate_downloaded_archive(path: &Path, extension: &str) -> Result<(), String> {
    match extension {
        "zip" => {
            let file = File::open(path).map_err(error_to_string)?;
            let archive = ZipArchive::new(file).map_err(|_| {
                "The download provider returned an incomplete or invalid ZIP archive. Start the download again."
                    .to_string()
            })?;
            if archive.is_empty() {
                return Err(
                    "The download provider returned an empty ZIP archive. Start the download again."
                        .to_string(),
                );
            }
            Ok(())
        }
        "7z" => validate_archive_signature(path, &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C], "7Z"),
        "rar" => {
            let mut file = File::open(path).map_err(error_to_string)?;
            let mut header = [0_u8; 8];
            let read = file.read(&mut header).map_err(error_to_string)?;
            let rar4 = read >= 7 && header[..7] == [0x52, 0x61, 0x72, 0x21, 0x1A, 0x07, 0x00];
            let rar5 = read >= 8 && header == [0x52, 0x61, 0x72, 0x21, 0x1A, 0x07, 0x01, 0x00];
            if rar4 || rar5 {
                Ok(())
            } else {
                Err(
                    "The download provider returned an incomplete or invalid RAR archive. Start the download again."
                        .to_string(),
                )
            }
        }
        _ => Ok(()),
    }
}

fn validate_archive_signature(path: &Path, signature: &[u8], label: &str) -> Result<(), String> {
    let mut file = File::open(path).map_err(error_to_string)?;
    let mut header = vec![0_u8; signature.len()];
    let valid = file
        .read_exact(&mut header)
        .map(|_| header == signature)
        .unwrap_or(false);
    if valid {
        Ok(())
    } else {
        Err(format!(
            "The download provider returned an incomplete or invalid {label} archive. Start the download again."
        ))
    }
}

fn validate_https_url(value: &str) -> Result<url::Url, String> {
    let parsed =
        url::Url::parse(value).map_err(|error| format!("Invalid download URL: {error}"))?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err("Downloads must use a valid HTTPS URL.".to_string());
    }
    Ok(parsed)
}

fn thunderstore_client() -> Result<Client, String> {
    provider_client()
}

fn provider_client() -> Result<Client, String> {
    Client::builder()
        .user_agent(format!("UniLoader/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(error_to_string)
}

fn thunderstore_package_id(package_ref: &ThunderstorePackageRef) -> String {
    format!(
        "thunderstore:{}/{}",
        package_ref.namespace, package_ref.name
    )
}

fn thunderstore_dependency_string(package_ref: &ThunderstorePackageRef, version: &str) -> String {
    format!("{}-{}-{}", package_ref.namespace, package_ref.name, version)
}

fn thunderstore_visit_key(package_ref: &ThunderstorePackageRef) -> String {
    format!(
        "thunderstore:{}:{}:{}",
        package_ref.namespace.to_lowercase(),
        package_ref.name.to_lowercase(),
        package_ref.version.as_deref().unwrap_or("*").to_lowercase()
    )
}

fn extract_href_values(html: &str) -> Vec<String> {
    html.split("href=\"")
        .skip(1)
        .filter_map(|part| part.split('"').next())
        .map(|href| href.replace("&amp;", "&"))
        .collect()
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    let pattern = pattern.to_lowercase();
    let value = value.to_lowercase();

    if pattern == "*" {
        return true;
    }

    if !pattern.contains('*') {
        return pattern == value;
    }

    let parts = pattern.split('*').collect::<Vec<_>>();
    let mut remaining = value.as_str();
    let mut first = true;

    for part in &parts {
        if part.is_empty() {
            continue;
        }

        if first && !pattern.starts_with('*') {
            if !remaining.starts_with(part) {
                return false;
            }
            remaining = &remaining[part.len()..];
        } else if let Some(index) = remaining.find(part) {
            remaining = &remaining[index + part.len()..];
        } else {
            return false;
        }

        first = false;
    }

    if !pattern.ends_with('*') {
        if let Some(last_part) = parts.iter().rev().find(|part| !part.is_empty()) {
            return value.ends_with(last_part);
        }
    }

    true
}

fn minimal_url_decode(value: &str) -> String {
    value
        .replace("%2B", "+")
        .replace("%2b", "+")
        .replace("%20", " ")
        .replace("%5B", "[")
        .replace("%5b", "[")
        .replace("%5D", "]")
        .replace("%5d", "]")
}

fn sanitize_file_segment(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn sanitize_url_path_segment(value: &str) -> String {
    sanitize_file_segment(value).replace('.', "")
}

fn materialize_import_source(
    store_root: &Path,
    profile: &GameProfile,
    install_id: &str,
    source_path: &Path,
) -> Result<PathBuf, String> {
    if !source_path.exists() {
        return Err(format!(
            "Import source no longer exists: {}",
            source_path.to_string_lossy()
        ));
    }

    let package_root = profile_package_dir(store_root, &profile.id, install_id);
    fs::create_dir_all(&package_root).map_err(error_to_string)?;

    if source_path.is_dir() {
        let destination = package_root.join("source");
        copy_dir_contents(source_path, &destination)?;
        return Ok(destination);
    }

    if !source_path.is_file() {
        return Err("Import source must be a file or folder.".to_string());
    }

    let file_name = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(sanitize_file_segment)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "source.zip".to_string());
    let destination = package_root.join(file_name);
    fs::copy(source_path, &destination).map_err(error_to_string)?;
    Ok(destination)
}

fn copy_dir_contents(source_root: &Path, destination_root: &Path) -> Result<(), String> {
    fs::create_dir_all(destination_root).map_err(error_to_string)?;
    let mut queue = VecDeque::from([source_root.to_path_buf()]);

    while let Some(current_path) = queue.pop_front() {
        for entry in fs::read_dir(&current_path).map_err(error_to_string)? {
            let entry = entry.map_err(error_to_string)?;
            let file_type = entry.file_type().map_err(error_to_string)?;
            if file_type.is_symlink() {
                continue;
            }

            let source_path = entry.path();
            let relative_path = source_path
                .strip_prefix(source_root)
                .map_err(error_to_string)?;
            let destination_path = safe_join(destination_root, &to_portable_path(relative_path))?;

            if file_type.is_dir() {
                fs::create_dir_all(&destination_path).map_err(error_to_string)?;
                queue.push_back(source_path);
                continue;
            }

            if file_type.is_file() {
                if let Some(parent) = destination_path.parent() {
                    fs::create_dir_all(parent).map_err(error_to_string)?;
                }
                fs::copy(&source_path, &destination_path).map_err(error_to_string)?;
            }
        }
    }

    Ok(())
}

#[derive(Debug)]
struct PreparedDeploymentFile {
    staged_path: PathBuf,
    destination: PathBuf,
    target_relative_path: String,
    is_game_file: bool,
}

fn deploy_plan_transaction(
    store_root: &Path,
    profile: &GameProfile,
    install_id: &str,
    source_path: &Path,
    plan: &InstallPlan,
    excluded_mod_id: Option<&str>,
) -> Result<DeploymentOutcome, String> {
    if plan.mappings.is_empty() {
        return Err("The selected install plan does not contain any files.".to_string());
    }
    if plan.mappings.len() > MAX_ARCHIVE_ENTRIES {
        return Err(format!(
            "The install plan contains too many files (maximum {}).",
            MAX_ARCHIVE_ENTRIES
        ));
    }

    let transaction_root = profile_dir(store_root, &profile.id)
        .join("transactions")
        .join(Uuid::new_v4().to_string());
    let stage_root = transaction_root.join("stage");
    let rollback_root = transaction_root.join("rollback");
    fs::create_dir_all(&stage_root).map_err(error_to_string)?;
    fs::create_dir_all(&rollback_root).map_err(error_to_string)?;

    let result = (|| -> Result<DeploymentOutcome, String> {
        let mut archive = if source_path.is_dir() {
            None
        } else {
            let archive_file = File::open(source_path).map_err(error_to_string)?;
            Some(ZipArchive::new(archive_file).map_err(error_to_string)?)
        };
        let profile_root = profile_dir(store_root, &profile.id);
        let mut prepared = Vec::with_capacity(plan.mappings.len());
        let mut destination_keys = HashSet::new();
        let mut expanded_bytes = 0_u64;

        for (index, mapping) in plan.mappings.iter().enumerate() {
            validate_archive_relative_path(&mapping.source_path)?;
            validate_archive_relative_path(&mapping.target_relative_path)?;

            let target_root = if mapping.target_root == "game" {
                PathBuf::from(&profile.game_path)
            } else {
                profile_root.clone()
            };
            let destination = safe_join(&target_root, &mapping.target_relative_path)?;
            let destination_key =
                normalize_filesystem_identity(destination.to_string_lossy().as_ref());
            if !destination_keys.insert(destination_key.clone()) {
                return Err(format!(
                    "The install plan maps more than one file to {}.",
                    destination.to_string_lossy()
                ));
            }

            if mapping.target_root == "game" {
                if let Some(owner) =
                    enabled_file_owner(store_root, &profile.id, &destination_key, excluded_mod_id)?
                {
                    return Err(format!(
                        "File conflict: {} is already managed by {}. Disable or remove that mod first.",
                        mapping.target_relative_path,
                        display_record_name(&owner)
                    ));
                }
            }

            let staged_path = stage_root.join(format!("{index}.bin"));
            let file_size = if let Some(archive) = archive.as_mut() {
                let mut zip_file = archive
                    .by_name(&mapping.source_path)
                    .map_err(|_| format!("Archive entry is missing: {}", mapping.source_path))?;
                validate_archive_file_size(
                    &mapping.source_path,
                    zip_file.size(),
                    zip_file.compressed_size(),
                )?;
                let mut output = File::create(&staged_path).map_err(error_to_string)?;
                copy_with_limit(&mut zip_file, &mut output, MAX_ARCHIVE_FILE_BYTES)?
            } else {
                let source_file = safe_join(source_path, &mapping.source_path)?;
                if !source_file.is_file() {
                    return Err(format!("Folder entry is missing: {}", mapping.source_path));
                }
                let size = source_file.metadata().map_err(error_to_string)?.len();
                validate_archive_file_size(&mapping.source_path, size, size)?;
                let mut input = File::open(&source_file).map_err(error_to_string)?;
                let mut output = File::create(&staged_path).map_err(error_to_string)?;
                copy_with_limit(&mut input, &mut output, MAX_ARCHIVE_FILE_BYTES)?
            };
            expanded_bytes = expanded_bytes
                .checked_add(file_size)
                .ok_or_else(|| "The expanded mod size overflowed its safety limit.".to_string())?;
            if expanded_bytes > MAX_ARCHIVE_EXPANDED_BYTES {
                return Err(format!(
                    "The mod expands beyond UniLoader's {} GB safety limit.",
                    MAX_ARCHIVE_EXPANDED_BYTES / 1024 / 1024 / 1024
                ));
            }

            prepared.push(PreparedDeploymentFile {
                staged_path,
                destination,
                target_relative_path: mapping.target_relative_path.clone(),
                is_game_file: mapping.target_root == "game",
            });
        }

        let backup_root = profile_backup_dir(store_root, &profile.id, install_id);
        let mut files_written = Vec::with_capacity(prepared.len());
        let mut backups_written = Vec::new();
        let mut written_file_hashes = HashMap::new();
        let mut rollback_entries = Vec::new();

        for (index, file) in prepared.iter().enumerate() {
            let immediate_backup = if file.destination.exists() {
                let backup = rollback_root.join(format!("{index}.bin"));
                fs::copy(&file.destination, &backup).map_err(error_to_string)?;
                Some(backup)
            } else {
                None
            };
            rollback_entries.push(DeploymentRollbackEntry {
                destination: file.destination.clone(),
                immediate_backup,
            });

            if file.is_game_file && file.destination.exists() {
                let backup_path = safe_join(&backup_root, &file.target_relative_path)?;
                if !backup_path.exists() {
                    if let Some(parent) = backup_path.parent() {
                        fs::create_dir_all(parent).map_err(error_to_string)?;
                    }
                    fs::copy(&file.destination, &backup_path).map_err(error_to_string)?;
                }
                backups_written.push(backup_path.to_string_lossy().to_string());
            }

            if let Err(error) = replace_file_from_path(&file.staged_path, &file.destination) {
                let outcome = DeploymentOutcome {
                    files_written,
                    backups_written,
                    written_file_hashes,
                    transaction_root: transaction_root.clone(),
                    rollback_entries,
                };
                outcome.rollback();
                return Err(error);
            }

            let destination_string = file.destination.to_string_lossy().to_string();
            written_file_hashes.insert(destination_string.clone(), sha256_file(&file.destination)?);
            files_written.push(destination_string);
        }

        Ok(DeploymentOutcome {
            files_written,
            backups_written,
            written_file_hashes,
            transaction_root: transaction_root.clone(),
            rollback_entries,
        })
    })();

    if result.is_err() && transaction_root.exists() {
        let _ = fs::remove_dir_all(&transaction_root);
    }
    result
}

fn enabled_file_owner(
    store_root: &Path,
    profile_id: &str,
    destination_key: &str,
    excluded_mod_id: Option<&str>,
) -> Result<Option<InstalledModRecord>, String> {
    let store = read_store::<InstalledModRecord>(&installed_mods_path(store_root))
        .map_err(error_to_string)?;
    Ok(store.items.into_iter().find(|record| {
        record.profile_id == profile_id
            && record.enabled
            && excluded_mod_id != Some(record.id.as_str())
            && record
                .files_written
                .iter()
                .any(|path| normalize_filesystem_identity(path) == destination_key)
    }))
}

fn validate_archive_relative_path(path: &str) -> Result<(), String> {
    let normalized = normalize_archive_path(path);
    let depth = Path::new(&normalized).components().count();
    if depth > MAX_ARCHIVE_PATH_DEPTH {
        return Err(format!(
            "Archive path is nested too deeply (maximum {}): {}",
            MAX_ARCHIVE_PATH_DEPTH, path
        ));
    }
    if normalized.len() > 1024 {
        return Err("An archive path exceeds UniLoader's safety limit.".to_string());
    }
    Ok(())
}

fn validate_archive_file_size(path: &str, size: u64, compressed_size: u64) -> Result<(), String> {
    if size > MAX_ARCHIVE_FILE_BYTES {
        return Err(format!("Archive file is too large: {path}"));
    }
    if compressed_size > 0 && size / compressed_size.max(1) > MAX_ARCHIVE_COMPRESSION_RATIO {
        return Err(format!(
            "Archive file has an unsafe compression ratio: {path}"
        ));
    }
    Ok(())
}

fn copy_with_limit<R: Read + ?Sized, W: Write>(
    reader: &mut R,
    writer: &mut W,
    limit: u64,
) -> Result<u64, String> {
    let mut limited = reader.take(limit.saturating_add(1));
    let copied = io::copy(&mut limited, writer).map_err(error_to_string)?;
    if copied > limit {
        return Err(format!(
            "A file exceeded UniLoader's {} MB safety limit.",
            limit / 1024 / 1024
        ));
    }
    Ok(copied)
}

fn replace_file_from_path(source: &Path, destination: &Path) -> Result<(), String> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(error_to_string)?;
    }
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("mod-file");
    let temporary = destination.with_file_name(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
    let result = (|| -> Result<(), String> {
        fs::copy(source, &temporary).map_err(|error| {
            format!(
                "Could not stage {} for {}: {error}",
                source.to_string_lossy(),
                destination.to_string_lossy()
            )
        })?;
        File::options()
            .read(true)
            .write(true)
            .open(&temporary)
            .and_then(|file| file.sync_all())
            .map_err(|error| format!("Could not flush {}: {error}", temporary.to_string_lossy()))?;
        if destination.exists() {
            fs::remove_file(destination).map_err(|error| {
                format!(
                    "Could not replace existing file {}: {error}",
                    destination.to_string_lossy()
                )
            })?;
        }
        fs::rename(&temporary, destination).map_err(|error| {
            format!(
                "Could not activate {}: {error}",
                destination.to_string_lossy()
            )
        })
    })();
    if temporary.exists() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(error_to_string)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(error_to_string)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn cleanup_install_data(store_root: &Path, profile_id: &str, install_id: &str) {
    let _ = fs::remove_dir_all(profile_package_dir(store_root, profile_id, install_id));
    let _ = fs::remove_dir_all(profile_backup_dir(store_root, profile_id, install_id));
    let receipt = profile_dir(store_root, profile_id)
        .join("receipts")
        .join(format!("{install_id}.json"));
    let _ = fs::remove_file(receipt);
}

fn restore_deactivated_mods(
    store_root: &Path,
    profile: &GameProfile,
    records: &[InstalledModRecord],
) {
    for record in records {
        let Some(plan) = record.plan.clone() else {
            continue;
        };
        let mut restored = record.clone();
        let install_id = restored.id.clone();
        let archive_path = restored.archive_path.clone();
        let _ = deploy_mod_files(
            store_root,
            profile,
            &install_id,
            &archive_path,
            &plan,
            &mut restored,
        );
    }
}

fn validate_profile_migration_plans(
    profile: &GameProfile,
    records: &[InstalledModRecord],
) -> Result<(), String> {
    let mut destinations = HashMap::<String, String>::new();
    for record in records
        .iter()
        .filter(|record| record.profile_id == profile.id && record.enabled)
    {
        let plan = record.plan.as_ref().ok_or_else(|| {
            format!(
                "{} cannot be migrated because its install plan is unavailable.",
                display_record_name(record)
            )
        })?;
        if let Some(reason) = incompatible_install_plan_reason(profile, plan) {
            return Err(format!("{}: {reason}", display_record_name(record)));
        }
        for mapping in plan
            .mappings
            .iter()
            .filter(|mapping| mapping.target_root == "game")
        {
            let destination =
                safe_join(Path::new(&profile.game_path), &mapping.target_relative_path)?;
            let key = normalize_filesystem_identity(destination.to_string_lossy().as_ref());
            if let Some(owner) = destinations.insert(key, display_record_name(record)) {
                return Err(format!(
                    "{} and {} both target {}. Resolve this conflict before changing folders.",
                    owner,
                    display_record_name(record),
                    mapping.target_relative_path
                ));
            }
        }
    }
    Ok(())
}

#[allow(dead_code)]
fn rollback_profile_migration(
    store_root: &Path,
    old_profile: &GameProfile,
    new_profile: &GameProfile,
    migrated_records: &[InstalledModRecord],
    old_records: &[InstalledModRecord],
) {
    for record in migrated_records.iter().rev() {
        let _ = deactivate_mod_files(store_root, new_profile, record);
        let _ = fs::remove_dir_all(profile_backup_dir(store_root, &new_profile.id, &record.id));
    }
    restore_deactivated_mods(store_root, old_profile, old_records);
    for record in old_records {
        let _ = write_receipt(store_root, old_profile, record);
    }
}

fn prepare_profile_mod_launch(
    store_root: &Path,
    profile: &GameProfile,
    mods_enabled: bool,
) -> Result<usize, String> {
    if mods_enabled {
        resume_profile_mod_files(store_root, profile)
    } else {
        suspend_profile_mod_files(store_root, profile)
    }
}

fn suspend_profile_mod_files(store_root: &Path, profile: &GameProfile) -> Result<usize, String> {
    let store = read_store::<InstalledModRecord>(&installed_mods_path(store_root))
        .map_err(error_to_string)?;
    let mut suspension = read_profile_launch_suspension(store_root, &profile.id)?;
    let original_suspension = suspension.clone();
    let mut suspended_records = Vec::<(InstalledModRecord, SuspendedLaunchMod)>::new();

    for record in store.items.iter().filter(|record| {
        record.profile_id == profile.id && record.enabled && record.runtime_id.is_none()
    }) {
        let has_active_files = record
            .files_written
            .iter()
            .any(|file_path| Path::new(file_path).is_file());
        if !has_active_files {
            continue;
        }

        let suspended = if let Some(existing) = suspension
            .mods
            .iter()
            .find(|item| item.mod_id == record.id)
            .cloned()
        {
            existing
        } else {
            let staged_files = if installed_record_can_redeploy(record) {
                Vec::new()
            } else {
                match stage_launch_mod_files(store_root, profile, record) {
                    Ok(files) => files,
                    Err(error) => {
                        rollback_launch_suspension(store_root, profile, &suspended_records);
                        cleanup_new_launch_snapshots(store_root, &original_suspension, &suspension);
                        return Err(format!(
                            "Could not prepare a clean launch for {}: {error}",
                            display_record_name(record)
                        ));
                    }
                }
            };
            let suspended = SuspendedLaunchMod {
                mod_id: record.id.clone(),
                staged_files,
            };
            suspension.mods.push(suspended.clone());
            suspended
        };

        if let Err(error) = deactivate_mod_files(store_root, profile, record) {
            rollback_launch_suspension(store_root, profile, &suspended_records);
            cleanup_new_launch_snapshots(store_root, &original_suspension, &suspension);
            return Err(format!(
                "Could not prepare a clean launch because {} could not be suspended: {error}",
                display_record_name(record)
            ));
        }
        suspended_records.push((record.clone(), suspended));
    }

    if let Err(error) = write_profile_launch_suspension(store_root, &suspension) {
        rollback_launch_suspension(store_root, profile, &suspended_records);
        cleanup_new_launch_snapshots(store_root, &original_suspension, &suspension);
        return Err(error);
    }

    Ok(suspended_records.len())
}

fn resume_profile_mod_files(store_root: &Path, profile: &GameProfile) -> Result<usize, String> {
    let suspension = read_profile_launch_suspension(store_root, &profile.id)?;
    if suspension.mods.is_empty() {
        return Ok(0);
    }

    let store = read_store::<InstalledModRecord>(&installed_mods_path(store_root))
        .map_err(error_to_string)?;
    let mut restored_records = Vec::<InstalledModRecord>::new();

    for suspended in &suspension.mods {
        let Some(record) = store.items.iter().find(|record| {
            record.id == suspended.mod_id
                && record.profile_id == profile.id
                && record.enabled
                && record.runtime_id.is_none()
        }) else {
            continue;
        };

        if let Err(error) = restore_suspended_launch_mod(store_root, profile, record, suspended) {
            for restored in restored_records.iter().rev() {
                let _ = deactivate_mod_files(store_root, profile, restored);
            }
            return Err(format!(
                "Could not restore {} for this launch: {error}",
                display_record_name(record)
            ));
        }
        restored_records.push(record.clone());
    }

    if let Err(error) = clear_profile_launch_suspension(store_root, &profile.id) {
        for restored in restored_records.iter().rev() {
            let _ = deactivate_mod_files(store_root, profile, restored);
        }
        return Err(error);
    }

    Ok(restored_records.len())
}

fn installed_record_can_redeploy(record: &InstalledModRecord) -> bool {
    record.plan.is_some() && Path::new(&record.archive_path).exists()
}

fn stage_launch_mod_files(
    store_root: &Path,
    profile: &GameProfile,
    record: &InstalledModRecord,
) -> Result<Vec<SuspendedLaunchFile>, String> {
    let suspension_root = profile_launch_suspension_dir(store_root, &profile.id);
    let files_root = suspension_root.join("files");
    fs::create_dir_all(&files_root).map_err(error_to_string)?;
    let mut staged = Vec::new();

    let result = (|| -> Result<(), String> {
        for destination in &record.files_written {
            let destination_path = Path::new(destination);
            if !destination_path.exists() {
                continue;
            }
            if !destination_path.is_file()
                || fs::symlink_metadata(destination_path)
                    .map_err(error_to_string)?
                    .file_type()
                    .is_symlink()
                || !path_is_inside(destination_path, Path::new(&profile.game_path))
            {
                return Err(format!(
                    "UniLoader will not stage an unsafe mod path: {}",
                    destination_path.to_string_lossy()
                ));
            }

            let snapshot_relative_path = format!("files/{}.bin", Uuid::new_v4());
            let snapshot_path = safe_join(&suspension_root, &snapshot_relative_path)?;
            fs::copy(destination_path, &snapshot_path).map_err(|error| {
                format!(
                    "Could not stage {} for a clean launch: {error}",
                    destination_path.to_string_lossy()
                )
            })?;
            staged.push(SuspendedLaunchFile {
                destination: destination.clone(),
                snapshot_relative_path,
            });
        }
        Ok(())
    })();

    if let Err(error) = result {
        remove_staged_launch_files(store_root, &profile.id, &staged);
        return Err(error);
    }

    Ok(staged)
}

fn restore_suspended_launch_mod(
    store_root: &Path,
    profile: &GameProfile,
    record: &InstalledModRecord,
    suspended: &SuspendedLaunchMod,
) -> Result<(), String> {
    if suspended.staged_files.is_empty() {
        let plan = record.plan.clone().ok_or_else(|| {
            "The managed source and install plan are no longer available.".to_string()
        })?;
        if !Path::new(&record.archive_path).exists() {
            return Err("The managed mod source is no longer available.".to_string());
        }
        let mut restored = record.clone();
        let install_id = restored.id.clone();
        let archive_path = restored.archive_path.clone();
        deploy_mod_files(
            store_root,
            profile,
            &install_id,
            &archive_path,
            &plan,
            &mut restored,
        )?;
        return Ok(());
    }

    let suspension_root = profile_launch_suspension_dir(store_root, &profile.id);
    for staged in &suspended.staged_files {
        let snapshot_path = safe_join(&suspension_root, &staged.snapshot_relative_path)?;
        if !snapshot_path.is_file() {
            return Err(format!(
                "A staged mod file is missing: {}",
                snapshot_path.to_string_lossy()
            ));
        }
        let destination = PathBuf::from(&staged.destination);
        if !launch_destination_is_inside_game(&destination, Path::new(&profile.game_path)) {
            return Err(format!(
                "A staged destination is outside the game folder: {}",
                destination.to_string_lossy()
            ));
        }
        replace_file_from_path(&snapshot_path, &destination)?;
    }
    Ok(())
}

fn launch_destination_is_inside_game(destination: &Path, game_path: &Path) -> bool {
    if destination.exists() {
        return path_is_inside(destination, game_path);
    }
    destination
        .parent()
        .is_some_and(|parent| path_is_inside(parent, game_path))
}

fn rollback_launch_suspension(
    store_root: &Path,
    profile: &GameProfile,
    suspended_records: &[(InstalledModRecord, SuspendedLaunchMod)],
) {
    for (record, suspended) in suspended_records.iter().rev() {
        let _ = restore_suspended_launch_mod(store_root, profile, record, suspended);
    }
}

fn cleanup_new_launch_snapshots(
    store_root: &Path,
    original: &ProfileLaunchSuspension,
    current: &ProfileLaunchSuspension,
) {
    let original_paths = original
        .mods
        .iter()
        .flat_map(|item| item.staged_files.iter())
        .map(|file| file.snapshot_relative_path.as_str())
        .collect::<HashSet<_>>();
    let new_files = current
        .mods
        .iter()
        .flat_map(|item| item.staged_files.iter())
        .filter(|file| !original_paths.contains(file.snapshot_relative_path.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    remove_staged_launch_files(store_root, &current.profile_id, &new_files);
}

fn remove_staged_launch_files(
    store_root: &Path,
    profile_id: &str,
    staged_files: &[SuspendedLaunchFile],
) {
    let suspension_root = profile_launch_suspension_dir(store_root, profile_id);
    for staged in staged_files {
        if let Ok(path) = safe_join(&suspension_root, &staged.snapshot_relative_path) {
            let _ = fs::remove_file(path);
        }
    }
}

fn read_profile_launch_suspension(
    store_root: &Path,
    profile_id: &str,
) -> Result<ProfileLaunchSuspension, String> {
    let path = profile_launch_suspension_manifest_path(store_root, profile_id);
    if !path.exists() {
        return Ok(ProfileLaunchSuspension {
            version: PROFILE_LAUNCH_SUSPENSION_VERSION,
            profile_id: profile_id.to_string(),
            mods: Vec::new(),
        });
    }

    let content = fs::read_to_string(&path).map_err(error_to_string)?;
    let suspension = parse_json_allow_bom::<ProfileLaunchSuspension>(&content)
        .map_err(|error| format!("Could not read the clean-launch state: {error}"))?;
    if suspension.version != PROFILE_LAUNCH_SUSPENSION_VERSION
        || suspension.profile_id != profile_id
    {
        return Err("The saved clean-launch state is not valid for this profile.".to_string());
    }
    Ok(suspension)
}

fn write_profile_launch_suspension(
    store_root: &Path,
    suspension: &ProfileLaunchSuspension,
) -> Result<(), String> {
    if suspension.mods.is_empty() {
        return Ok(());
    }
    let path = profile_launch_suspension_manifest_path(store_root, &suspension.profile_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(error_to_string)?;
    }
    let content = serde_json::to_string_pretty(suspension).map_err(error_to_string)?;
    atomic_write(&path, format!("{content}\n").as_bytes()).map_err(error_to_string)
}

fn clear_profile_launch_suspension(store_root: &Path, profile_id: &str) -> Result<(), String> {
    let suspension_root = profile_launch_suspension_dir(store_root, profile_id);
    if !suspension_root.exists() {
        return Ok(());
    }
    let cleared =
        suspension_root.with_file_name(format!("launch-suspension-cleared-{}", Uuid::new_v4()));
    fs::rename(&suspension_root, &cleared)
        .map_err(|error| format!("Could not finish restoring the selected mods: {error}"))?;
    let _ = fs::remove_dir_all(cleared);
    Ok(())
}

fn deactivate_mod_files(
    store_root: &Path,
    profile: &GameProfile,
    record: &InstalledModRecord,
) -> Result<Vec<String>, String> {
    let backup_root = profile_backup_dir(store_root, &profile.id, &record.id);
    let transaction_root = profile_dir(store_root, &profile.id)
        .join("transactions")
        .join(Uuid::new_v4().to_string());
    fs::create_dir_all(&transaction_root).map_err(error_to_string)?;

    let mut affected_paths = record
        .files_written
        .iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    for backup_path in &record.backups_written {
        let backup = PathBuf::from(backup_path);
        if let Ok(relative_path) = backup.strip_prefix(&backup_root) {
            let target = safe_join(
                Path::new(&profile.game_path),
                &to_portable_path(relative_path),
            )?;
            if !affected_paths.iter().any(|path| path == &target) {
                affected_paths.push(target);
            }
        }
    }

    for file_path in &record.files_written {
        let path = PathBuf::from(file_path);
        if !path.exists() {
            continue;
        }
        if let Some(expected_hash) = record.written_file_hashes.get(file_path) {
            let current_hash = sha256_file(&path)?;
            if &current_hash != expected_hash {
                let _ = fs::remove_dir_all(&transaction_root);
                return Err(format!(
                    "UniLoader did not disable {} because it was modified after installation: {}",
                    display_record_name(record),
                    path.to_string_lossy()
                ));
            }
        }
    }

    let mut snapshots = Vec::with_capacity(affected_paths.len());
    for (index, path) in affected_paths.iter().enumerate() {
        let snapshot = if path.exists() {
            let snapshot = transaction_root.join(format!("{index}.bin"));
            fs::copy(path, &snapshot).map_err(error_to_string)?;
            Some(snapshot)
        } else {
            None
        };
        snapshots.push(DeploymentRollbackEntry {
            destination: path.clone(),
            immediate_backup: snapshot,
        });
    }

    let operation = (|| -> Result<Vec<String>, String> {
        let mut files_changed = Vec::new();
        for file_path in &record.files_written {
            let path = PathBuf::from(file_path);
            if path.exists() {
                fs::remove_file(&path).map_err(error_to_string)?;
                files_changed.push(path.to_string_lossy().to_string());
            }
        }

        for backup_path in &record.backups_written {
            let backup = PathBuf::from(backup_path);
            if !backup.exists() {
                continue;
            }
            let Ok(relative_path) = backup.strip_prefix(&backup_root) else {
                continue;
            };
            let target = safe_join(
                Path::new(&profile.game_path),
                &to_portable_path(relative_path),
            )?;
            replace_file_from_path(&backup, &target)?;
            files_changed.push(target.to_string_lossy().to_string());
        }
        Ok(files_changed)
    })();

    match operation {
        Ok(files_changed) => {
            let _ = fs::remove_dir_all(transaction_root);
            Ok(files_changed)
        }
        Err(error) => {
            for snapshot in snapshots.iter().rev() {
                if let Some(source) = &snapshot.immediate_backup {
                    let _ = replace_file_from_path(source, &snapshot.destination);
                } else if snapshot.destination.exists() {
                    let _ = fs::remove_file(&snapshot.destination);
                }
            }
            let _ = fs::remove_dir_all(transaction_root);
            Err(error)
        }
    }
}

fn deploy_mod_files(
    store_root: &Path,
    profile: &GameProfile,
    install_id: &str,
    archive_path: &str,
    plan: &InstallPlan,
    record: &mut InstalledModRecord,
) -> Result<Vec<String>, String> {
    let source_path = Path::new(archive_path);
    let deployment = deploy_plan_transaction(
        store_root,
        profile,
        install_id,
        source_path,
        plan,
        Some(&record.id),
    )?;
    record.files_written = deployment.files_written.clone();
    record.backups_written = deployment.backups_written.clone();
    record.written_file_hashes = deployment.written_file_hashes.clone();
    let files_written = deployment.files_written.clone();
    deployment.commit();
    Ok(files_written)
}

fn adopt_existing_native_script_mods(
    store_root: &Path,
    profile: &GameProfile,
    store: &mut StoreFile<InstalledModRecord>,
) -> Result<usize, String> {
    let script_files = discover_native_script_files(profile);
    if script_files.is_empty() {
        return Ok(0);
    }

    let existing_targets = store
        .items
        .iter()
        .filter(|record| record.profile_id == profile.id && record.last_status != "removed")
        .flat_map(|record| record.files_written.iter())
        .map(|path| normalize_filesystem_identity(path))
        .collect::<HashSet<_>>();

    let mut adopted_count = 0;
    for script_file in script_files {
        let target_identity =
            normalize_filesystem_identity(script_file.absolute_path.to_string_lossy().as_ref());
        if existing_targets.contains(&target_identity) {
            continue;
        }

        let install_id = Uuid::new_v4().to_string();
        let source_root = profile_package_dir(store_root, &profile.id, &install_id).join("source");
        let managed_source_path = safe_join(&source_root, &script_file.source_relative_path)?;
        if let Some(parent) = managed_source_path.parent() {
            fs::create_dir_all(parent).map_err(error_to_string)?;
        }
        fs::copy(&script_file.absolute_path, &managed_source_path).map_err(error_to_string)?;

        let plan = InstallPlan {
            adapter_id: "script-files".to_string(),
            adapter_name: "Native Script Mods".to_string(),
            confidence: 1.0,
            summary: format!(
                "Track existing script mod at {}.",
                script_file.target_relative_path
            ),
            mappings: vec![mapping(
                &script_file.source_relative_path,
                "game",
                &script_file.target_relative_path,
                "Existing game-side script mod adopted by UniLoader.",
            )],
            dependencies: Vec::new(),
            warnings: Vec::new(),
            requires_confirmation: false,
        };
        let display_name = humanize_mod_display_name(&script_file.source_relative_path);
        let installed_at = now_string();
        let record = InstalledModRecord {
            id: install_id,
            profile_id: profile.id.clone(),
            archive_path: source_root.to_string_lossy().to_string(),
            archive_name: basename(&script_file.source_relative_path),
            display_name: Some(display_name),
            package_id: Some(format!(
                "external-script:{}",
                script_file.target_relative_path.to_lowercase()
            )),
            dependency_string: None,
            icon_url: None,
            adapter_id: "script-files".to_string(),
            summary: format!(
                "Existing script mod tracked from {}.",
                script_file.target_relative_path
            ),
            installed_at,
            files_written: vec![script_file.absolute_path.to_string_lossy().to_string()],
            backups_written: Vec::new(),
            written_file_hashes: HashMap::from([(
                script_file.absolute_path.to_string_lossy().to_string(),
                sha256_file(&script_file.absolute_path)?,
            )]),
            dependencies: Vec::new(),
            config_files: Vec::new(),
            runtime_id: None,
            externally_managed: false,
            enabled: true,
            last_status: "installed".to_string(),
            plan: Some(plan),
        };

        write_receipt(store_root, profile, &record)?;
        store.items.push(record);
        adopted_count += 1;
    }

    Ok(adopted_count)
}

fn ensure_visible_runtime_records(
    store_root: &Path,
    profile: &GameProfile,
    store: &mut StoreFile<InstalledModRecord>,
) -> Result<usize, String> {
    let mut changed = 0;
    for runtime in profile_runtime_ids(profile) {
        if !runtime_installed(profile, &runtime) {
            continue;
        }

        if let Some(record) = store.items.iter_mut().find(|record| {
            record.profile_id == profile.id
                && record.last_status != "removed"
                && runtime_record_matches(record, &runtime)
        }) {
            let mut record_changed = false;
            if record.runtime_id.as_deref() != Some(runtime.as_str()) {
                record.runtime_id = Some(runtime.clone());
                record_changed = true;
            }
            let display_name = format_loader(&runtime);
            if record.display_name.as_deref() != Some(display_name) {
                record.display_name = Some(display_name.to_string());
                record_changed = true;
            }
            if record_changed {
                record.summary = format!(
                    "System runtime required by {} mods.",
                    profile_game_label(profile)
                );
                write_receipt(store_root, profile, record)?;
                changed += 1;
            }
            continue;
        }

        let files_written = detected_runtime_signature_files(profile, &runtime);
        let installed_at = now_string();
        let record = InstalledModRecord {
            id: Uuid::new_v4().to_string(),
            profile_id: profile.id.clone(),
            archive_path: profile.game_path.clone(),
            archive_name: format_loader(&runtime).to_string(),
            display_name: Some(format_loader(&runtime).to_string()),
            package_id: Some(format!("observed-runtime:{runtime}")),
            dependency_string: None,
            icon_url: None,
            adapter_id: runtime_adapter_id(&runtime).to_string(),
            summary: format!(
                "System runtime detected in the {} game folder.",
                profile_game_label(profile)
            ),
            installed_at,
            files_written: files_written.clone(),
            backups_written: Vec::new(),
            written_file_hashes: files_written
                .iter()
                .filter_map(|path| {
                    sha256_file(Path::new(path))
                        .ok()
                        .map(|hash| (path.clone(), hash))
                })
                .collect(),
            dependencies: Vec::new(),
            config_files: Vec::new(),
            runtime_id: Some(runtime),
            externally_managed: true,
            enabled: true,
            last_status: "installed".to_string(),
            plan: None,
        };
        write_receipt(store_root, profile, &record)?;
        store.items.push(record);
        changed += 1;
    }
    Ok(changed)
}

fn runtime_record_matches(record: &InstalledModRecord, runtime: &str) -> bool {
    record
        .runtime_id
        .as_deref()
        .is_some_and(|record_runtime| record_runtime.eq_ignore_ascii_case(runtime))
        || record
            .files_written
            .iter()
            .any(|path| runtime_signature_file_matches(runtime, path))
}

fn detected_runtime_signature_files(profile: &GameProfile, runtime: &str) -> Vec<String> {
    let game_path = Path::new(&profile.game_path);
    let mut files = walk_game_folder(game_path)
        .into_iter()
        .filter(|entry| {
            !entry.is_directory && runtime_signature_file_matches(runtime, &entry.relative_path)
        })
        .map(|entry| {
            game_path
                .join(entry.relative_path)
                .to_string_lossy()
                .to_string()
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|path| path.to_lowercase());
    files.dedup_by(|first, second| first.eq_ignore_ascii_case(second));
    files
}

fn runtime_signature_file_matches(runtime: &str, path: &str) -> bool {
    let normalized = normalize_archive_path(path).to_lowercase();
    let name = basename(&normalized).to_lowercase();
    match runtime {
        "bepinex" | "bepinex-il2cpp" => normalized.ends_with("bepinex/core/bepinex.dll"),
        "ue4ss" => matches!(name.as_str(), "ue4ss.dll" | "ue4ss-settings.ini"),
        "reframework" => matches!(name.as_str(), "dinput8.dll" | "reframework_revision.txt"),
        _ => false,
    }
}

fn runtime_adapter_id(runtime: &str) -> &str {
    match runtime {
        "bepinex" | "bepinex-il2cpp" => "bepinex",
        "ue4ss" => "ue4ss",
        "reframework" => "reframework",
        _ => "loose-files",
    }
}

fn discover_native_script_files(profile: &GameProfile) -> Vec<NativeScriptFile> {
    let game_path = Path::new(&profile.game_path);
    let mut files = Vec::new();
    let mut seen = HashSet::new();

    for route in native_script_target_dirs(profile) {
        let Ok(root) = safe_join(game_path, &route) else {
            continue;
        };
        if root.is_dir() {
            discover_native_script_files_in_dir(&root, &route, 0, &mut files, &mut seen);
        }
    }

    files.sort_by_key(|file| file.target_relative_path.to_lowercase());
    files
}

fn discover_native_script_files_in_dir(
    root: &Path,
    route: &str,
    depth: usize,
    files: &mut Vec<NativeScriptFile>,
    seen: &mut HashSet<String>,
) {
    if depth > MAX_CONFIG_SCAN_DEPTH {
        return;
    }

    let Ok(entries) = fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };

        if file_type.is_dir() {
            discover_native_script_files_in_dir(&path, route, depth + 1, files, seen);
            continue;
        }

        if !file_type.is_file()
            || !path
                .extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| extension.eq_ignore_ascii_case("as"))
                .unwrap_or(false)
        {
            continue;
        }

        let Ok(relative_path) = path.strip_prefix(root) else {
            continue;
        };
        let source_relative_path = to_portable_path(relative_path);
        let target_relative_path =
            normalize_archive_path(&format!("{}/{}", route, source_relative_path));
        let identity = normalize_filesystem_identity(path.to_string_lossy().as_ref());
        if seen.insert(identity) {
            files.push(NativeScriptFile {
                absolute_path: path,
                target_relative_path,
                source_relative_path,
            });
        }
    }
}

fn normalize_filesystem_identity(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
}

fn normalize_profile_game_path(path: &str) -> String {
    let trimmed = path.trim();
    #[cfg(target_os = "windows")]
    {
        trimmed.replace('/', "\\")
    }
    #[cfg(not(target_os = "windows"))]
    {
        trimmed.to_string()
    }
}

fn discover_profile_config_files(profile: &GameProfile) -> Vec<String> {
    let mut files = Vec::new();
    let mut seen = HashSet::new();

    for root in profile_config_roots(profile) {
        if root.is_dir() {
            discover_config_files_in_dir(&root, 0, &mut files, &mut seen);
        } else if root.is_file() && is_user_editable_config_file(&root) {
            push_unique_config_path(&mut files, &mut seen, &root);
        }

        if files.len() >= MAX_PROFILE_CONFIG_FILES {
            break;
        }
    }

    files.sort_by_key(|path| path.to_lowercase());
    files
}

fn profile_config_roots(profile: &GameProfile) -> Vec<PathBuf> {
    let game_path = Path::new(&profile.game_path);
    let mut roots = vec![
        game_path.join("BepInEx").join("config"),
        game_path.join("MelonLoader").join("UserData"),
        game_path.join("UserData"),
        game_path.join("config"),
        game_path.join("Config"),
        game_path.join("Configs"),
        game_path.join("Mods"),
        game_path.join("mods"),
        game_path.join("UE4SS"),
        game_path.join("REFramework"),
        game_path.join("reframework"),
        game_path.join("doorstop_config.ini"),
        game_path.join("UE4SS-settings.ini"),
        game_path.join("REFramework.ini"),
        game_path.join("dinput8.ini"),
    ];

    if let Some(definition) = profile.game_id.as_deref().and_then(game_definition_by_id) {
        roots.extend(
            definition
                .config_roots
                .iter()
                .filter_map(|root| safe_join(game_path, root).ok()),
        );
    }

    roots
}

fn discover_config_files_in_dir(
    root: &Path,
    depth: usize,
    files: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    if depth > MAX_CONFIG_SCAN_DEPTH || files.len() >= MAX_PROFILE_CONFIG_FILES {
        return;
    }

    let Ok(entries) = fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        if files.len() >= MAX_PROFILE_CONFIG_FILES {
            break;
        }

        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };

        if file_type.is_file() {
            if is_user_editable_config_file(&path) {
                push_unique_config_path(files, seen, &path);
            }
        } else if file_type.is_dir() && should_descend_config_dir(&path) {
            discover_config_files_in_dir(&path, depth + 1, files, seen);
        }
    }
}

fn should_descend_config_dir(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_lowercase();

    !matches!(
        name.as_str(),
        ".git" | "cache" | "core" | "logs" | "plugins" | "tmp" | "temp"
    ) && !is_localization_directory_name(&name)
}

fn push_unique_config_path(files: &mut Vec<String>, seen: &mut HashSet<String>, path: &Path) {
    let key = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase();

    if seen.insert(key) {
        files.push(path.to_string_lossy().to_string());
    }
}

fn is_supported_config_file(path: &Path) -> bool {
    let Some(extension) = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_lowercase())
    else {
        return false;
    };

    matches!(
        extension.as_str(),
        "cfg" | "conf" | "config" | "ini" | "json" | "toml" | "yaml" | "yml"
    )
}

fn is_user_editable_config_file(path: &Path) -> bool {
    is_supported_config_file(path) && !is_localization_resource_path(path)
}

fn is_localization_resource_path(path: &Path) -> bool {
    let components = path
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>();
    if components.len() > 1 {
        let directories = &components[..components.len() - 1];
        let subtree_start = directories
            .iter()
            .rposition(|component| is_config_tree_boundary(component))
            .map(|index| index + 1)
            .unwrap_or_else(|| directories.len().saturating_sub(3));
        if directories[subtree_start..]
            .iter()
            .any(|component| is_localization_directory_name(component))
        {
            return true;
        }
    }

    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let tokens = stem
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .collect::<Vec<_>>();

    tokens.iter().any(|token| is_language_resource_token(token))
}

fn is_config_tree_boundary(name: &str) -> bool {
    let compact = name
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .collect::<String>();
    matches!(
        compact.as_str(),
        "config" | "configs" | "mods" | "userdata" | "ue4ss" | "reframework" | "melonloader"
    )
}

fn is_localization_directory_name(name: &str) -> bool {
    let compact = name
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .collect::<String>();
    compact.contains("translation")
        || compact.contains("localization")
        || matches!(
            compact.as_str(),
            "i18n" | "l10n" | "lang" | "langs" | "language" | "languages" | "locale" | "locales"
        )
}

fn is_language_resource_token(token: &str) -> bool {
    matches!(
        token,
        "arabic"
            | "brazilian"
            | "chinese"
            | "czech"
            | "danish"
            | "dutch"
            | "english"
            | "finnish"
            | "french"
            | "german"
            | "hungarian"
            | "indonesian"
            | "italian"
            | "japanese"
            | "korean"
            | "norwegian"
            | "polish"
            | "portuguese"
            | "romanian"
            | "russian"
            | "spanish"
            | "swedish"
            | "thai"
            | "traditionalchinese"
            | "simplifiedchinese"
            | "turkish"
            | "ukrainian"
            | "vietnamese"
            | "en"
            | "enus"
            | "engb"
            | "zh"
            | "zhcn"
            | "zhtw"
            | "ja"
            | "jp"
            | "ko"
            | "kr"
            | "de"
            | "fr"
            | "es"
            | "it"
            | "pt"
            | "ptbr"
            | "ru"
            | "pl"
    )
}

fn resolved_config_files_for_record(
    profile: &GameProfile,
    record: &InstalledModRecord,
    discovered_config_files: &[String],
) -> Vec<String> {
    let mut files = Vec::new();
    let mut seen = HashSet::new();

    for path in &record.config_files {
        let path = Path::new(path);
        if path.is_file() && is_user_editable_config_file(path) {
            push_unique_config_path(&mut files, &mut seen, path);
        }
    }

    for path in discovered_config_files {
        if is_user_editable_config_file(Path::new(path))
            && config_file_matches_record(profile, record, path)
        {
            push_unique_config_path(&mut files, &mut seen, Path::new(path));
        }
    }

    files.sort_by_key(|path| path.to_lowercase());
    files
}

fn config_file_matches_record(
    profile: &GameProfile,
    record: &InstalledModRecord,
    config_path: &str,
) -> bool {
    let filename = Path::new(config_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(config_path);
    let lower_filename = filename.to_lowercase();

    if runtime_config_filename_matches(profile, record, &lower_filename) {
        return true;
    }

    let config_key = config_match_key(filename);
    if config_key.is_empty() {
        return false;
    }

    config_match_needles(record).iter().any(|needle| {
        needle.len() >= 4
            && (config_key.contains(needle)
                || (config_key.len() >= 8 && needle.contains(&config_key)))
    })
}

fn runtime_config_filename_matches(
    profile: &GameProfile,
    record: &InstalledModRecord,
    lower_filename: &str,
) -> bool {
    if !record_is_runtime_package(profile, record) {
        return false;
    }

    if record.adapter_id.contains("bepinex") {
        return matches!(lower_filename, "bepinex.cfg" | "doorstop_config.ini");
    }

    if record.adapter_id == "ue4ss" {
        return lower_filename == "ue4ss-settings.ini";
    }

    if record.adapter_id == "reframework" {
        return matches!(lower_filename, "reframework.ini" | "dinput8.ini");
    }

    false
}

fn record_is_runtime_package(profile: &GameProfile, record: &InstalledModRecord) -> bool {
    let combined = [
        record.display_name.as_deref().unwrap_or_default(),
        &record.archive_name,
        record.package_id.as_deref().unwrap_or_default(),
        record.dependency_string.as_deref().unwrap_or_default(),
    ]
    .join(" ")
    .to_lowercase();

    combined.contains("bepinexpack")
        || combined.contains("bepinex pack")
        || combined.contains("reframework")
        || combined.contains("ue4ss")
        || profile.loader == record.adapter_id
            && record.files_written.iter().any(|path| {
                let lower = basename(&normalize_archive_path(path)).to_lowercase();
                matches!(
                    lower.as_str(),
                    "winhttp.dll"
                        | "doorstop_config.ini"
                        | "dinput8.dll"
                        | "ue4ss.dll"
                        | "reframework.dll"
                )
            })
}

fn config_match_needles(record: &InstalledModRecord) -> Vec<String> {
    let mut values = Vec::new();

    if let Some(display_name) = record.display_name.as_deref() {
        values.push(display_name.to_string());
    }
    values.push(record.archive_name.clone());

    if let Some(package_id) = record.package_id.as_deref() {
        values.extend(package_id.split([':', '/', '-']).map(str::to_string));
    }

    if let Some(dependency_string) = record.dependency_string.as_deref() {
        values.extend(dependency_string.split('-').map(str::to_string));
    }

    for path in &record.files_written {
        values.push(file_stem_from_path(path));
    }

    let mut needles = Vec::new();
    let mut seen = HashSet::new();
    for value in values {
        let key = config_match_key(&humanize_mod_display_name(&value));
        if key.len() >= 4 && !is_generic_config_needle(&key) && seen.insert(key.clone()) {
            needles.push(key);
        }
    }

    needles
}

fn file_stem_from_path(path: &str) -> String {
    let normalized_path = normalize_archive_path(path);
    let file_name = basename(&normalized_path);
    Path::new(&file_name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(&file_name)
        .to_string()
}

fn config_match_key(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

fn is_generic_config_needle(needle: &str) -> bool {
    matches!(
        needle,
        "bepinex"
            | "config"
            | "cfg"
            | "game"
            | "main"
            | "manual"
            | "mod"
            | "mods"
            | "pack"
            | "plugin"
            | "plugins"
            | "release"
            | "unity"
            | "valheim"
            | "version"
    )
}

fn read_mod_config_file(
    store_root: &Path,
    profile: &GameProfile,
    file_path: &str,
) -> Result<ModConfigFile, String> {
    let path = PathBuf::from(file_path);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(file_path)
        .to_string();

    let warning_file = |warning: String| ModConfigFile {
        path: file_path.to_string(),
        file_name: file_name.clone(),
        entries: Vec::new(),
        raw_preview: String::new(),
        warning: Some(warning),
    };

    if !path.exists() {
        return Ok(warning_file("Config file no longer exists.".to_string()));
    }

    if !path.is_file() {
        return Ok(warning_file(
            "Config path exists, but it is not a file.".to_string(),
        ));
    }

    if is_localization_resource_path(&path) {
        return Ok(warning_file(
            "This is a translation/localization resource, not a user setting.".to_string(),
        ));
    }

    let game_path = Path::new(&profile.game_path);
    if !path_is_inside(&path, game_path) && !path_is_inside(&path, store_root) {
        return Ok(warning_file(
            "Config file is outside the selected game/profile folders and was not read."
                .to_string(),
        ));
    }

    let metadata = fs::metadata(&path).map_err(error_to_string)?;
    if metadata.len() > MAX_CONFIG_READ_BYTES {
        return Ok(warning_file(format!(
            "Config file is larger than {} KB and was not read.",
            MAX_CONFIG_READ_BYTES / 1024
        )));
    }

    let content = fs::read_to_string(&path).map_err(error_to_string)?;
    Ok(ModConfigFile {
        path: file_path.to_string(),
        file_name,
        entries: parse_mod_config_entries(&path, &content),
        raw_preview: content.chars().take(6000).collect(),
        warning: None,
    })
}

fn path_is_inside(path: &Path, root: &Path) -> bool {
    let Ok(path) = path.canonicalize() else {
        return false;
    };
    let Ok(root) = root.canonicalize() else {
        return false;
    };

    path.starts_with(root)
}

fn validate_config_file_for_edit(
    store_root: &Path,
    profile: &GameProfile,
    path: &Path,
) -> Result<(), String> {
    if !path.exists() {
        return Err("Config file no longer exists.".to_string());
    }

    if !path.is_file() {
        return Err("Config path exists, but it is not a file.".to_string());
    }

    if fs::symlink_metadata(path)
        .map_err(error_to_string)?
        .file_type()
        .is_symlink()
    {
        return Err("Symbolic-link config files are not edited for safety.".to_string());
    }

    if !is_supported_config_file(path) {
        return Err("This file type is not editable as a config file yet.".to_string());
    }
    if is_localization_resource_path(path) {
        return Err(
            "Translation/localization resources are not exposed as user settings.".to_string(),
        );
    }

    let game_path = Path::new(&profile.game_path);
    if !path_is_inside(path, game_path) && !path_is_inside(path, store_root) {
        return Err(
            "Config file is outside the selected game/profile folders and was not changed."
                .to_string(),
        );
    }

    let metadata = fs::metadata(path).map_err(error_to_string)?;
    if metadata.len() > MAX_CONFIG_READ_BYTES {
        return Err(format!(
            "Config file is larger than {} KB and was not changed.",
            MAX_CONFIG_READ_BYTES / 1024
        ));
    }

    Ok(())
}

fn parse_mod_config_entries(path: &Path, content: &str) -> Vec<ModConfigEntry> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_lowercase())
        .as_deref()
    {
        Some("json") => parse_json_config_entries(content),
        Some("toml") => parse_toml_config_entries(content),
        Some("yaml") | Some("yml") => parse_yaml_config_entries(content),
        _ => parse_key_value_config_entries(content),
    }
}

fn parse_json_config_entries(content: &str) -> Vec<ModConfigEntry> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return parse_key_value_config_entries(content);
    };
    let mut entries = Vec::new();
    flatten_json_config_value(&value, Vec::new(), &mut entries);
    entries
}

fn flatten_json_config_value(
    value: &serde_json::Value,
    path: Vec<String>,
    entries: &mut Vec<ModConfigEntry>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                let mut next_path = path.clone();
                next_path.push(key.clone());
                flatten_json_config_value(value, next_path, entries);
            }
        }
        _ => {
            let key = path.last().cloned().unwrap_or_else(|| "value".to_string());
            let section = if path.len() > 1 {
                Some(path[..path.len() - 1].join("."))
            } else {
                None
            };
            entries.push(ModConfigEntry {
                section,
                key,
                value: json_config_value_display(value),
                value_type: Some(json_config_value_type(value).to_string()),
                default_value: None,
                description: None,
            });
        }
    }
}

fn json_config_value_display(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(value) => value.clone(),
        _ => value.to_string(),
    }
}

fn json_config_value_type(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

fn parse_toml_config_entries(content: &str) -> Vec<ModConfigEntry> {
    let Ok(document) = content.parse::<toml_edit::DocumentMut>() else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    flatten_toml_table(document.as_table(), Vec::new(), &mut entries);
    entries
}

fn flatten_toml_table(
    table: &toml_edit::Table,
    path: Vec<String>,
    entries: &mut Vec<ModConfigEntry>,
) {
    for (key, item) in table.iter() {
        let mut next_path = path.clone();
        next_path.push(key.to_string());
        match item {
            toml_edit::Item::Table(nested) => {
                flatten_toml_table(nested, next_path, entries);
            }
            toml_edit::Item::Value(value) => {
                let section = if path.is_empty() {
                    None
                } else {
                    Some(path.join("."))
                };
                entries.push(ModConfigEntry {
                    section,
                    key: key.to_string(),
                    value: toml_config_value_display(value),
                    value_type: Some(value.type_name().to_string()),
                    default_value: None,
                    description: None,
                });
            }
            toml_edit::Item::ArrayOfTables(_) | toml_edit::Item::None => {}
        }
    }
}

fn toml_config_value_display(value: &toml_edit::Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string().trim().to_string())
}

fn update_toml_config_content(
    content: &str,
    section: Option<&str>,
    key: &str,
    next_value: &str,
) -> Result<String, String> {
    let mut document = content
        .parse::<toml_edit::DocumentMut>()
        .map_err(error_to_string)?;
    let mut current = document.as_item_mut();
    for part in section
        .unwrap_or_default()
        .split('.')
        .filter(|part| !part.trim().is_empty())
    {
        current = current
            .get_mut(part)
            .ok_or_else(|| format!("Config section not found: {part}"))?;
    }
    let target = current
        .get_mut(key)
        .and_then(toml_edit::Item::as_value_mut)
        .ok_or_else(|| format!("Setting not found: {key}"))?;
    *target = typed_toml_config_value(target, next_value)?;
    Ok(document.to_string())
}

fn typed_toml_config_value(
    current: &toml_edit::Value,
    next_value: &str,
) -> Result<toml_edit::Value, String> {
    if current.is_str() {
        return Ok(toml_edit::Value::from(next_value));
    }

    let candidate = format!("value = {}", next_value.trim())
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| format!("Invalid TOML value: {error}"))?
        .get("value")
        .and_then(toml_edit::Item::as_value)
        .cloned()
        .ok_or_else(|| "Invalid TOML value.".to_string())?;

    if candidate.type_name() != current.type_name() {
        return Err(format!(
            "This setting requires a {} value.",
            current.type_name()
        ));
    }
    Ok(candidate)
}

fn parse_yaml_config_entries(content: &str) -> Vec<ModConfigEntry> {
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(content) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    flatten_yaml_config_value(&value, Vec::new(), &mut entries);
    entries
}

fn flatten_yaml_config_value(
    value: &serde_yaml::Value,
    path: Vec<String>,
    entries: &mut Vec<ModConfigEntry>,
) {
    if let serde_yaml::Value::Mapping(map) = value {
        for (key, value) in map {
            let Some(key) = key.as_str() else {
                continue;
            };
            let mut next_path = path.clone();
            next_path.push(key.to_string());
            flatten_yaml_config_value(value, next_path, entries);
        }
        return;
    }

    let key = path.last().cloned().unwrap_or_else(|| "value".to_string());
    let section = if path.len() > 1 {
        Some(path[..path.len() - 1].join("."))
    } else {
        None
    };
    entries.push(ModConfigEntry {
        section,
        key,
        value: yaml_config_value_display(value),
        value_type: Some(yaml_config_value_type(value).to_string()),
        default_value: None,
        description: None,
    });
}

fn yaml_config_value_display(value: &serde_yaml::Value) -> String {
    match value {
        serde_yaml::Value::String(value) => value.clone(),
        _ => serde_yaml::to_string(value)
            .unwrap_or_default()
            .trim()
            .to_string(),
    }
}

fn yaml_config_value_type(value: &serde_yaml::Value) -> &'static str {
    match value {
        serde_yaml::Value::Null => "null",
        serde_yaml::Value::Bool(_) => "bool",
        serde_yaml::Value::Number(_) => "number",
        serde_yaml::Value::String(_) => "string",
        serde_yaml::Value::Sequence(_) => "array",
        serde_yaml::Value::Mapping(_) => "object",
        serde_yaml::Value::Tagged(_) => "tagged",
    }
}

fn update_yaml_config_content(
    content: &str,
    section: Option<&str>,
    key: &str,
    next_value: &str,
) -> Result<String, String> {
    let mut root = serde_yaml::from_str::<serde_yaml::Value>(content).map_err(error_to_string)?;
    let mut path = section
        .map(|section| {
            section
                .split('.')
                .filter(|part| !part.trim().is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    path.push(key.to_string());

    let target = yaml_value_at_path_mut(&mut root, &path)
        .ok_or_else(|| format!("Setting not found: {key}"))?;
    *target = typed_yaml_config_value(target, next_value)?;
    serde_yaml::to_string(&root).map_err(error_to_string)
}

fn yaml_value_at_path_mut<'a>(
    value: &'a mut serde_yaml::Value,
    path: &[String],
) -> Option<&'a mut serde_yaml::Value> {
    let mut current = value;
    for part in path {
        let serde_yaml::Value::Mapping(map) = current else {
            return None;
        };
        current = map.get_mut(serde_yaml::Value::String(part.clone()))?;
    }
    Some(current)
}

fn typed_yaml_config_value(
    current: &serde_yaml::Value,
    next_value: &str,
) -> Result<serde_yaml::Value, String> {
    if matches!(current, serde_yaml::Value::String(_)) {
        return Ok(serde_yaml::Value::String(next_value.to_string()));
    }
    let candidate =
        serde_yaml::from_str::<serde_yaml::Value>(next_value).map_err(error_to_string)?;
    if yaml_config_value_type(&candidate) != yaml_config_value_type(current) {
        return Err(format!(
            "This setting requires a {} value.",
            yaml_config_value_type(current)
        ));
    }
    Ok(candidate)
}

fn validate_structured_config_content(
    extension: Option<&str>,
    content: &str,
) -> Result<(), String> {
    match extension {
        Some("json") => serde_json::from_str::<serde_json::Value>(content)
            .map(|_| ())
            .map_err(error_to_string),
        Some("toml") => content
            .parse::<toml_edit::DocumentMut>()
            .map(|_| ())
            .map_err(error_to_string),
        Some("yaml") | Some("yml") => serde_yaml::from_str::<serde_yaml::Value>(content)
            .map(|_| ())
            .map_err(error_to_string),
        _ => Ok(()),
    }
}

fn update_json_config_content(
    content: &str,
    section: Option<&str>,
    key: &str,
    next_value: &str,
) -> Result<String, String> {
    let mut root = serde_json::from_str::<serde_json::Value>(content).map_err(error_to_string)?;
    let mut path = section
        .map(|section| {
            section
                .split('.')
                .filter(|part| !part.trim().is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    path.push(key.to_string());

    let Some(target) = json_value_at_path_mut(&mut root, &path) else {
        return Err(format!("Setting not found: {}", key));
    };
    *target = json_config_update_value(target, next_value)?;

    serde_json::to_string_pretty(&root).map_err(error_to_string)
}

fn json_value_at_path_mut<'a>(
    value: &'a mut serde_json::Value,
    path: &[String],
) -> Option<&'a mut serde_json::Value> {
    let mut current = value;
    for part in path {
        let serde_json::Value::Object(map) = current else {
            return None;
        };
        current = map.get_mut(part)?;
    }

    Some(current)
}

fn json_config_update_value(
    current_value: &serde_json::Value,
    next_value: &str,
) -> Result<serde_json::Value, String> {
    match current_value {
        serde_json::Value::Bool(_) => match next_value.trim().to_lowercase().as_str() {
            "true" => Ok(serde_json::Value::Bool(true)),
            "false" => Ok(serde_json::Value::Bool(false)),
            _ => Err("Boolean settings must be true or false.".to_string()),
        },
        serde_json::Value::Number(_) => serde_json::from_str::<serde_json::Value>(next_value)
            .map_err(error_to_string)
            .and_then(|value| {
                if value.is_number() {
                    Ok(value)
                } else {
                    Err("Number settings must be saved as a number.".to_string())
                }
            }),
        serde_json::Value::String(_) => Ok(serde_json::Value::String(next_value.to_string())),
        serde_json::Value::Null => Ok(serde_json::Value::String(next_value.to_string())),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            serde_json::from_str::<serde_json::Value>(next_value).map_err(error_to_string)
        }
    }
}

fn update_key_value_config_content(
    content: &str,
    target_section: Option<&str>,
    target_key: &str,
    next_value: &str,
) -> Result<String, String> {
    let newline = if content.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let had_trailing_newline = content.ends_with('\n') || content.ends_with('\r');
    let normalized_target_section = target_section.map(normalized_config_label);
    let normalized_target_key = normalized_config_label(target_key);
    let mut current_section: Option<String> = None;
    let mut updated = false;
    let mut lines = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(section) = config_section_name(trimmed) {
            current_section = Some(section);
            lines.push(line.to_string());
            continue;
        }

        if config_comment(trimmed).is_some() {
            lines.push(line.to_string());
            continue;
        }

        if !updated {
            if let Some(delimiter_index) = config_assignment_delimiter_index(line) {
                let key_part = line[..delimiter_index].trim().trim_matches('"');
                if normalized_config_label(key_part) == normalized_target_key
                    && config_section_matches(
                        current_section.as_deref(),
                        normalized_target_section.as_deref(),
                    )
                {
                    lines.push(replace_assignment_value(line, delimiter_index, next_value));
                    updated = true;
                    continue;
                }
            }
        }

        lines.push(line.to_string());
    }

    if !updated {
        return Err(format!("Setting not found: {}", target_key));
    }

    let mut output = lines.join(newline);
    if had_trailing_newline {
        output.push_str(newline);
    }

    Ok(output)
}

fn config_section_name(line: &str) -> Option<String> {
    if line.starts_with('[') && line.ends_with(']') {
        Some(
            line.trim_start_matches('[')
                .trim_end_matches(']')
                .trim()
                .to_string(),
        )
    } else {
        None
    }
}

fn config_assignment_delimiter_index(line: &str) -> Option<usize> {
    line.find('=').or_else(|| line.find(':'))
}

fn config_section_matches(current_section: Option<&str>, target_section: Option<&str>) -> bool {
    match target_section {
        Some(target_section) => current_section
            .map(normalized_config_label)
            .map(|current_section| current_section == target_section)
            .unwrap_or(false),
        None => current_section.is_none(),
    }
}

fn replace_assignment_value(line: &str, delimiter_index: usize, next_value: &str) -> String {
    let value_start = delimiter_index + 1;
    let whitespace_len = line[value_start..]
        .char_indices()
        .take_while(|(_, character)| character.is_whitespace())
        .map(|(index, character)| index + character.len_utf8())
        .last()
        .unwrap_or(0);
    let spacing = &line[value_start..value_start + whitespace_len];

    format!("{}{}{}", &line[..value_start], spacing, next_value.trim())
}

fn normalized_config_label(value: &str) -> String {
    value.trim().to_lowercase()
}

fn parse_key_value_config_entries(content: &str) -> Vec<ModConfigEntry> {
    let mut entries = Vec::new();
    let mut section = None;
    let mut pending_comments = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = Some(
                trimmed
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .trim()
                    .to_string(),
            );
            pending_comments.clear();
            continue;
        }

        if let Some(comment) = config_comment(trimmed) {
            if !comment.is_empty() {
                pending_comments.push(comment.to_string());
            }
            continue;
        }

        if let Some((key, value)) = split_config_assignment(trimmed) {
            let (value_type, default_value, description) =
                config_comment_metadata(&pending_comments);
            entries.push(ModConfigEntry {
                section: section.clone(),
                key: key.trim().trim_matches('"').to_string(),
                value: clean_config_value(value),
                value_type,
                default_value,
                description,
            });
            pending_comments.clear();
            continue;
        }

        pending_comments.clear();
    }

    entries
}

fn config_comment(line: &str) -> Option<&str> {
    line.strip_prefix("##")
        .or_else(|| line.strip_prefix('#'))
        .or_else(|| line.strip_prefix(';'))
        .map(str::trim)
}

fn split_config_assignment(line: &str) -> Option<(&str, &str)> {
    line.split_once('=').or_else(|| line.split_once(':'))
}

fn clean_config_value(value: &str) -> String {
    value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

fn config_comment_metadata(
    comments: &[String],
) -> (Option<String>, Option<String>, Option<String>) {
    let mut value_type = None;
    let mut default_value = None;
    let mut description = Vec::new();

    for comment in comments {
        let trimmed = comment.trim();
        if let Some(value) = strip_prefix_ignore_ascii_case(trimmed, "Setting type:") {
            value_type = Some(value.trim().to_string());
        } else if let Some(value) = strip_prefix_ignore_ascii_case(trimmed, "Default value:") {
            default_value = Some(value.trim().to_string());
        } else if trimmed.starts_with("Settings file was created")
            || trimmed.starts_with("Plugin GUID:")
        {
            continue;
        } else if !trimmed.is_empty() {
            description.push(trimmed.to_string());
        }
    }

    let description = if description.is_empty() {
        None
    } else {
        Some(description.join(" "))
    };

    (value_type, default_value, description)
}

fn strip_prefix_ignore_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    value
        .get(..prefix.len())
        .filter(|candidate| candidate.eq_ignore_ascii_case(prefix))
        .and_then(|_| value.get(prefix.len()..))
}

fn config_files_from_paths(paths: &[String]) -> Vec<String> {
    paths
        .iter()
        .filter(|path| {
            let lower_path = path.to_lowercase().replace('\\', "/");
            lower_path.contains("/config/")
                || lower_path.contains("/reframework/autorun/")
                || lower_path.ends_with(".cfg")
                || lower_path.ends_with(".ini")
                || lower_path.ends_with(".conf")
                || lower_path.ends_with(".config")
                || lower_path.ends_with(".json")
                || lower_path.ends_with(".toml")
                || lower_path.ends_with(".yaml")
                || lower_path.ends_with(".yml")
        })
        .cloned()
        .collect()
}

fn install_display_name(
    store_root: &Path,
    archive_path: &str,
    plan: &InstallPlan,
    metadata: &InstallMetadata,
) -> String {
    if let Some(display_name) = metadata.display_name.as_deref() {
        return humanize_mod_display_name(display_name);
    }

    if let Some(manifest_name) = import_manifest_name(store_root, Path::new(archive_path)) {
        return humanize_mod_display_name(&manifest_name);
    }

    if let Some(primary_source) = primary_mapping_source(plan) {
        return humanize_mod_display_name(&primary_source);
    }

    if let Some(archive_name) = metadata.archive_name.as_deref() {
        return humanize_mod_display_name(archive_name);
    }

    humanize_mod_display_name(
        Path::new(archive_path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(archive_path),
    )
}

fn import_manifest_name(store_root: &Path, source_path: &Path) -> Option<String> {
    let scanned = scan_import_source(store_root, source_path).ok()?;
    scanned.manifest.and_then(|manifest| {
        let trimmed = manifest.name.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn primary_mapping_source(plan: &InstallPlan) -> Option<String> {
    plan.mappings
        .iter()
        .find(|mapping| {
            let lower = mapping.source_path.to_lowercase();
            lower.ends_with(".pak")
                || lower.ends_with(".dll")
                || lower.ends_with(".lua")
                || lower.ends_with(".ucas")
                || lower.ends_with(".utoc")
        })
        .or_else(|| plan.mappings.first())
        .map(|mapping| basename(&mapping.source_path))
}

fn humanize_mod_display_name(raw_name: &str) -> String {
    let normalized_path = normalize_archive_path(raw_name);
    let file_name = basename(&normalized_path);
    let lower_file_name = file_name.to_lowercase();
    let without_extension = [".zip", ".7z", ".rar", ".pak", ".dll", ".lua", ".as"]
        .iter()
        .find_map(|extension| {
            lower_file_name
                .ends_with(extension)
                .then(|| &file_name[..file_name.len() - extension.len()])
        })
        .unwrap_or(&file_name);
    let without_unreal_suffix = without_extension
        .strip_suffix("_P")
        .or_else(|| without_extension.strip_suffix("-P"))
        .unwrap_or(without_extension);
    let spaced = without_unreal_suffix
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>();
    let mut words = spaced
        .split_whitespace()
        .flat_map(split_readable_mod_words)
        .collect::<Vec<_>>();

    let mut removed_hosting_tail = false;
    while let Some(last_word) = words.last() {
        if is_strong_mod_hosting_noise_token(last_word) {
            words.pop();
            removed_hosting_tail = true;
            continue;
        }

        if removed_hosting_tail
            && last_word
                .chars()
                .all(|character| character.is_ascii_digit())
        {
            words.pop();
            continue;
        }

        break;
    }

    if words.is_empty() {
        return "Unknown Mod".to_string();
    }

    polish_mod_display_name(&words.join(" "))
}

fn split_readable_mod_words(word: &str) -> Vec<String> {
    let characters = word.chars().collect::<Vec<_>>();
    let mut parts = Vec::new();
    let mut current = String::new();

    for (index, character) in characters.iter().enumerate() {
        if index > 0 {
            let previous = characters[index - 1];
            let next = characters.get(index + 1).copied();
            let camel_boundary = character.is_ascii_uppercase()
                && (previous.is_ascii_lowercase()
                    || previous.is_ascii_digit()
                    || (previous.is_ascii_uppercase()
                        && next
                            .map(|next_character| next_character.is_ascii_lowercase())
                            .unwrap_or(false)));
            let alpha_to_digit = character.is_ascii_digit() && previous.is_ascii_alphabetic();

            if (camel_boundary || alpha_to_digit) && !current.is_empty() {
                parts.push(current);
                current = String::new();
            }
        }

        current.push(*character);
    }

    if !current.is_empty() {
        parts.push(current);
    }

    parts
}

fn is_strong_mod_hosting_noise_token(token: &str) -> bool {
    let lower = token.to_lowercase();
    let has_digit = lower.chars().any(|character| character.is_ascii_digit());
    let has_alpha = lower
        .chars()
        .any(|character| character.is_ascii_alphabetic());
    lower.len() >= 8 && lower.chars().all(|character| character.is_ascii_hexdigit())
        || lower.len() >= 8 && has_digit
        || matches!(
            lower.as_str(),
            "manual" | "main" | "latest" | "version" | "release" | "file" | "files"
        )
        || (has_digit && has_alpha && lower.len() >= 6)
}

fn polish_mod_display_name(name: &str) -> String {
    let mut words = name
        .split_whitespace()
        .map(|word| word.to_string())
        .collect::<Vec<_>>();

    trim_known_file_extension_tail(&mut words);

    if words.len() > 1 && looks_like_mod_author_prefix(&words[0]) {
        words.remove(0);
    }

    while words.len() >= 3
        && words[words.len() - 3..]
            .iter()
            .all(|word| is_number_word(word))
    {
        words.truncate(words.len() - 3);
    }

    let mut name = words.join(" ");
    name = replace_ascii_case_insensitive(&name, "Bep In Ex", "BepInEx");
    name = replace_ascii_case_insensitive(&name, "BepInExPack", "BepInEx Pack");
    name = replace_ascii_case_insensitive(&name, "Ue 4 Ss", "UE4SS");
    name = replace_ascii_case_insensitive(&name, "Re Framework", "REFramework");

    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn trim_known_file_extension_tail(words: &mut Vec<String>) {
    if words.len() <= 1 {
        return;
    }

    let Some(last_word) = words.last() else {
        return;
    };

    if is_known_file_extension_word(last_word) {
        words.pop();
    }
}

fn is_known_file_extension_word(word: &str) -> bool {
    matches!(
        word.to_lowercase().as_str(),
        "as" | "lua" | "dll" | "pak" | "zip" | "rar" | "7z"
    )
}

fn looks_like_mod_author_prefix(word: &str) -> bool {
    let lower = word.to_lowercase();
    matches!(
        lower.as_str(),
        "denikson" | "thunderstore" | "nexusmods" | "nexus" | "overwolf"
    )
}

fn is_number_word(word: &str) -> bool {
    word.chars().all(|character| character.is_ascii_digit())
}

fn replace_ascii_case_insensitive(value: &str, needle: &str, replacement: &str) -> String {
    let mut result = String::new();
    let mut remaining = value;

    loop {
        let lower_remaining = remaining.to_lowercase();
        let lower_needle = needle.to_lowercase();
        let Some(index) = lower_remaining.find(&lower_needle) else {
            result.push_str(remaining);
            break;
        };

        result.push_str(&remaining[..index]);
        result.push_str(replacement);
        remaining = &remaining[index + needle.len()..];
    }

    result
}

fn default_enabled() -> bool {
    true
}

fn default_last_status() -> String {
    "installed".to_string()
}

fn installable_files(entries: &[ArchiveEntry]) -> Vec<ArchiveEntry> {
    entries
        .iter()
        .filter(|entry| !entry.is_directory && !is_package_metadata(&entry.logical_path))
        .cloned()
        .collect()
}

fn is_package_metadata(path: &str) -> bool {
    matches!(
        path.to_lowercase().as_str(),
        "manifest.json"
            | "readme.md"
            | "readme.txt"
            | "icon.png"
            | "changelog.md"
            | "license"
            | "license.txt"
    )
}

fn mapping(
    source_path: &str,
    target_root: &str,
    target_relative_path: &str,
    reason: &str,
) -> InstallMapping {
    InstallMapping {
        source_path: normalize_archive_path(source_path),
        target_root: target_root.to_string(),
        target_relative_path: normalize_archive_path(target_relative_path),
        reason: reason.to_string(),
    }
}

fn read_manifest(
    archive_path: &Path,
    entries: &[ArchiveEntry],
) -> Result<(Option<ThunderstoreManifest>, Option<PackageIdentity>), String> {
    let Some(manifest_entry) = entries.iter().find(|entry| {
        !entry.is_directory && entry.logical_path.eq_ignore_ascii_case("manifest.json")
    }) else {
        return Ok((None, None));
    };

    let archive_file = File::open(archive_path).map_err(error_to_string)?;
    let mut archive = ZipArchive::new(archive_file).map_err(error_to_string)?;
    let mut manifest_file = archive
        .by_name(&manifest_entry.path)
        .map_err(error_to_string)?;
    let mut manifest_content = String::new();
    io::Read::read_to_string(&mut manifest_file, &mut manifest_content).map_err(error_to_string)?;

    parse_embedded_package_manifest(&manifest_content)
}

fn read_folder_manifest(
    folder_path: &Path,
    entries: &[ArchiveEntry],
) -> Result<(Option<ThunderstoreManifest>, Option<PackageIdentity>), String> {
    let Some(manifest_entry) = entries.iter().find(|entry| {
        !entry.is_directory && entry.logical_path.eq_ignore_ascii_case("manifest.json")
    }) else {
        return Ok((None, None));
    };

    let manifest_path = safe_join(folder_path, &manifest_entry.path)?;
    let manifest_content = fs::read_to_string(manifest_path).map_err(error_to_string)?;
    parse_embedded_package_manifest(&manifest_content)
}

fn parse_embedded_package_manifest(
    manifest_content: &str,
) -> Result<(Option<ThunderstoreManifest>, Option<PackageIdentity>), String> {
    let value = parse_json_allow_bom::<serde_json::Value>(manifest_content)
        .map_err(|error| format!("Could not read package manifest.json: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "Package manifest.json must contain a JSON object.".to_string())?;

    let has_thunderstore_version =
        object.contains_key("version_number") || object.contains_key("versionNumber");
    if object.contains_key("name") && has_thunderstore_version {
        let manifest = serde_json::from_value::<ThunderstoreManifest>(value)
            .map_err(|error| format!("Invalid Thunderstore manifest.json: {error}"))?;
        let dependencies = manifest.dependencies.clone().unwrap_or_default();
        let identity = PackageIdentity {
            provider: "thunderstore".to_string(),
            package_id: Some(format!(
                "thunderstore-manifest:{}",
                compact_provider_slug(&manifest.name)
            )),
            version: Some(manifest.version_number.clone()),
            provider_game_id: None,
            mod_types: Vec::new(),
            dependencies,
            evidence: vec!["Thunderstore manifest.json".to_string()],
            confidence: 0.82,
        };
        return Ok((Some(manifest), Some(identity)));
    }

    let is_curseforge = object.contains_key("manifestType")
        || object.contains_key("manifestVersion")
        || (object.contains_key("minecraft") && object.contains_key("files"));
    if is_curseforge {
        let name = json_string_field(object, &["name"]);
        let version = json_string_field(object, &["version"]);
        let provider_game_id = if object.contains_key("minecraft") {
            Some("minecraft".to_string())
        } else {
            json_string_field(object, &["gameId", "game"])
        };
        let dependencies = object
            .get("files")
            .and_then(|files| files.as_array())
            .into_iter()
            .flatten()
            .filter_map(|file| {
                let project_id = file.get("projectID").or_else(|| file.get("projectId"))?;
                let file_id = file.get("fileID").or_else(|| file.get("fileId"));
                Some(match file_id {
                    Some(file_id) => format!("curseforge:{project_id}#{file_id}"),
                    None => format!("curseforge:{project_id}"),
                })
            })
            .collect::<Vec<_>>();
        let package_id = name
            .as_deref()
            .map(compact_provider_slug)
            .filter(|name| !name.is_empty())
            .map(|name| format!("curseforge-manifest:{name}"));
        return Ok((
            None,
            Some(PackageIdentity {
                provider: "curseforge".to_string(),
                package_id,
                version,
                provider_game_id,
                mod_types: Vec::new(),
                dependencies,
                evidence: vec!["CurseForge manifest.json".to_string()],
                confidence: 0.78,
            }),
        ));
    }

    Ok((None, None))
}

fn json_string_field(
    object: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<String> {
    keys.iter().find_map(|key| {
        object.get(*key).and_then(|value| match value {
            serde_json::Value::String(value) => non_empty_string(value.trim()),
            serde_json::Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
    })
}

fn normalize_archive_path(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches('/').to_string()
}

fn common_top_folder(paths: &[String]) -> Option<String> {
    let file_paths = paths
        .iter()
        .filter(|path| !path.ends_with('/'))
        .collect::<Vec<_>>();
    if file_paths.is_empty() {
        return None;
    }

    let first_parts = file_paths[0].split('/').collect::<Vec<_>>();
    if first_parts.len() < 2 {
        return None;
    }

    let candidate = first_parts[0];
    if file_paths
        .iter()
        .all(|path| path.starts_with(&format!("{}/", candidate)))
    {
        Some(candidate.to_string())
    } else {
        None
    }
}

fn to_logical_path(path: &str, common_top_folder: Option<&str>) -> String {
    if let Some(common_top_folder) = common_top_folder {
        let prefix = format!("{}/", common_top_folder);
        if path.starts_with(&prefix) {
            return path[prefix.len()..].to_string();
        }
    }

    path.to_string()
}

fn basename(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

fn folder_import_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("mod-folder")
        .to_string()
}

fn path_after_named_segment(path: &str, segment: &str) -> Option<String> {
    let target = segment.to_lowercase();
    let parts = normalize_archive_path(path)
        .split('/')
        .map(|part| part.to_string())
        .collect::<Vec<_>>();

    parts
        .iter()
        .position(|part| part.eq_ignore_ascii_case(&target))
        .and_then(|index| {
            let remaining = parts
                .iter()
                .skip(index + 1)
                .filter(|part| !part.is_empty())
                .cloned()
                .collect::<Vec<_>>();

            if remaining.is_empty() {
                None
            } else {
                Some(remaining.join("/"))
            }
        })
}

fn is_bepinex_root_runtime_file(path: &str) -> bool {
    matches!(
        basename(path).to_lowercase().as_str(),
        "doorstop_config.ini"
            | "winhttp.dll"
            | "doorstop_config_il2cpp.ini"
            | "winhttp_il2cpp.dll"
            | "start_game_bepinex.sh"
            | "run_bepinex.sh"
    )
}

fn is_probable_bepinex_plugin_dll(path: &str, profile: &GameProfile) -> bool {
    let file_name = basename(path).to_lowercase();
    if !file_name.ends_with(".dll") || is_known_native_bootstrap_file(&file_name) {
        return false;
    }

    profile.engine.starts_with("unity") || profile.engine == "unknown"
}

fn is_known_native_bootstrap_file(file_name: &str) -> bool {
    matches!(
        file_name,
        "dinput8.dll"
            | "xinput1_3.dll"
            | "dwmapi.dll"
            | "ue4ss.dll"
            | "openvr_api.dll"
            | "openxr_loader.dll"
            | "version.dll"
            | "winmm.dll"
    )
}

fn is_ue4ss_root_runtime_file(path: &str) -> bool {
    matches!(
        basename(path).to_lowercase().as_str(),
        "ue4ss.dll" | "ue4ss-settings.ini" | "dwmapi.dll" | "xinput1_3.dll"
    )
}

fn is_reframework_root_runtime_file(path: &str) -> bool {
    matches!(
        basename(path).to_lowercase().as_str(),
        "dinput8.dll"
            | "openvr_api.dll"
            | "openxr_loader.dll"
            | "reframework_revision.txt"
            | "delete_openvr_api_dll_if_you_want_to_use_openxr"
    )
}

fn native_script_target_dirs(profile: &GameProfile) -> Vec<String> {
    let game_path = Path::new(&profile.game_path);
    let entries = if game_path.is_dir() {
        walk_game_folder(game_path)
    } else {
        Vec::new()
    };

    native_script_routes_for_detection(profile.game_id.as_deref(), &entries)
}

fn bepinex_target_roots(profile: &GameProfile) -> Vec<String> {
    loader_target_roots(profile, "bepinex", "BepInEx")
}

fn reframework_target_roots(profile: &GameProfile) -> Vec<String> {
    loader_target_roots(profile, "reframework", "reframework")
}

fn loader_target_roots(profile: &GameProfile, loader_dir: &str, fallback: &str) -> Vec<String> {
    let game_path = Path::new(&profile.game_path);
    let entries = if game_path.is_dir() {
        walk_game_folder(game_path)
    } else {
        Vec::new()
    };
    detected_loader_roots(&entries, loader_dir, fallback)
}

fn install_route_parent(route: &str) -> String {
    normalize_archive_path(route)
        .rsplit_once('/')
        .map(|(parent, _)| parent.to_string())
        .unwrap_or_default()
}

fn join_install_route(parent: &str, child: &str) -> String {
    let parent = normalize_archive_path(parent).trim_matches('/').to_string();
    let child = normalize_archive_path(child).trim_matches('/').to_string();
    if parent.is_empty() {
        child
    } else if child.is_empty() {
        parent
    } else {
        format!("{parent}/{child}")
    }
}

fn ue4ss_install_targets(profile: &GameProfile) -> (Vec<String>, Vec<String>) {
    let game_path = Path::new(&profile.game_path);
    let entries = if game_path.is_dir() {
        walk_game_folder(game_path)
    } else {
        Vec::new()
    };
    let mut roots = find_unreal_win64_dirs(&entries);
    roots.sort_by(|left, right| {
        let left_server = left.to_lowercase().contains("/windowsserver/");
        let right_server = right.to_lowercase().contains("/windowsserver/");
        left_server
            .cmp(&right_server)
            .then(left.matches('/').count().cmp(&right.matches('/').count()))
            .then(left.len().cmp(&right.len()))
    });
    roots.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    if roots.is_empty() {
        roots.push("Binaries/Win64".to_string());
    }
    let mod_routes = ue4ss_mod_routes_for_entries(&entries, &roots);
    (roots, mod_routes)
}

fn native_script_payload_relative(path: &str) -> String {
    relative_after_native_script_mods(path)
        .or_else(|| {
            let normalized = normalize_archive_path(path);
            normalized
                .strip_prefix("Mods/")
                .or_else(|| normalized.strip_prefix("mods/"))
                .map(str::to_string)
        })
        .unwrap_or_else(|| basename(path))
}

fn relative_after_native_script_mods(path: &str) -> Option<String> {
    let parts = normalize_archive_path(path)
        .split('/')
        .map(|part| part.to_string())
        .collect::<Vec<_>>();

    parts
        .windows(2)
        .position(|window| {
            matches!(window[0].to_lowercase().as_str(), "script" | "scripts")
                && window[1].eq_ignore_ascii_case("mods")
        })
        .and_then(|index| {
            let remaining = parts
                .iter()
                .skip(index + 2)
                .filter(|part| !part.is_empty())
                .cloned()
                .collect::<Vec<_>>();

            if remaining.is_empty() {
                None
            } else {
                Some(remaining.join("/"))
            }
        })
}

fn unreal_pak_target_dirs(profile: &GameProfile) -> Vec<String> {
    let game_path = Path::new(&profile.game_path);
    let mut pak_roots = find_unreal_pak_roots(game_path);

    pak_roots.sort();
    pak_roots.dedup();

    pak_roots.sort_by(|left, right| {
        let left_server_penalty = if left.to_lowercase().contains("/windowsserver/") {
            1
        } else {
            0
        };
        let right_server_penalty = if right.to_lowercase().contains("/windowsserver/") {
            1
        } else {
            0
        };
        left_server_penalty
            .cmp(&right_server_penalty)
            .then(left.matches('/').count().cmp(&right.matches('/').count()))
            .then(left.len().cmp(&right.len()))
    });

    if pak_roots.is_empty() {
        vec!["Content/Paks/~mods".to_string()]
    } else {
        pak_roots
            .into_iter()
            .map(|root| format!("{}/~mods", root))
            .collect()
    }
}

fn find_unreal_pak_roots(game_path: &Path) -> Vec<String> {
    let mut pak_roots = Vec::new();
    let mut queue = VecDeque::from([(game_path.to_path_buf(), PathBuf::new(), 0usize)]);

    while let Some((absolute_path, relative_path, depth)) = queue.pop_front() {
        if depth > MAX_UNREAL_PAK_ROOT_SCAN_DEPTH || pak_roots.len() >= MAX_UNREAL_PAK_ROOTS {
            continue;
        }

        let Ok(dirents) = fs::read_dir(&absolute_path) else {
            continue;
        };

        for dirent in dirents.flatten() {
            if pak_roots.len() >= MAX_UNREAL_PAK_ROOTS {
                break;
            }

            let is_directory = dirent
                .file_type()
                .map(|file_type| file_type.is_dir())
                .unwrap_or(false);
            if !is_directory {
                continue;
            }

            let name = dirent.file_name().to_string_lossy().to_string();
            let child_relative_path = relative_path.join(&name);
            let child_depth = depth + 1;
            let portable_path = to_portable_path(&child_relative_path);
            let lower_path = portable_path.to_lowercase();

            if lower_path.ends_with("content/paks") && is_valid_unreal_game_pak_root(&lower_path) {
                pak_roots.push(portable_path.clone());
            }

            if should_descend_for_unreal_pak_roots(&portable_path, &name, child_depth) {
                queue.push_back((dirent.path(), child_relative_path, child_depth));
            }
        }
    }

    pak_roots
}

fn is_valid_unreal_game_pak_root(lower_path: &str) -> bool {
    !lower_path.starts_with("engine/")
        && !lower_path.contains("/engine/content/paks")
        && !lower_path.contains("/.git/")
        && !lower_path.contains("/node_modules/")
}

fn should_descend_for_unreal_pak_roots(relative_path: &str, name: &str, depth: usize) -> bool {
    if depth > MAX_UNREAL_PAK_ROOT_SCAN_DEPTH {
        return false;
    }

    let lower_name = name.to_lowercase();
    let lower_path = relative_path.to_lowercase();

    if matches!(
        lower_name.as_str(),
        "node_modules" | ".git" | "screenshots" | "captures" | "logs" | "crash reports"
    ) {
        return false;
    }

    depth <= 3
        || matches!(
            lower_name.as_str(),
            "builds" | "content" | "dedicatedserver" | "paks" | "r5" | "server" | "windowsserver"
        )
        || lower_name.ends_with("_data")
        || lower_path.contains("/builds")
        || lower_path.contains("/content")
        || lower_path.contains("/server")
        || lower_path.contains("/windowsserver")
}

fn join_human_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [single] => single.clone(),
        [first, second] => format!("{} and {}", first, second),
        _ => {
            let mut text = items[..items.len() - 1].join(", ");
            text.push_str(", and ");
            text.push_str(&items[items.len() - 1]);
            text
        }
    }
}

fn archive_stem(archive_name: &str) -> String {
    archive_name
        .strip_suffix(".zip")
        .or_else(|| archive_name.strip_suffix(".7z"))
        .or_else(|| archive_name.strip_suffix(".rar"))
        .unwrap_or(archive_name)
        .to_string()
}

fn walk_game_folder(root: &Path) -> Vec<ProbeEntry> {
    let mut entries = Vec::new();
    let mut queue = VecDeque::from([(root.to_path_buf(), PathBuf::new(), 0usize)]);

    while let Some((absolute_path, relative_path, depth)) = queue.pop_front() {
        if depth > MAX_SCAN_DEPTH || entries.len() >= MAX_SCAN_ENTRIES {
            continue;
        }

        let Ok(dirents) = fs::read_dir(&absolute_path) else {
            continue;
        };

        for dirent in dirents.flatten() {
            if entries.len() >= MAX_SCAN_ENTRIES {
                break;
            }

            let name = dirent.file_name().to_string_lossy().to_string();
            let child_relative_path = relative_path.join(&name);
            let is_directory = dirent
                .file_type()
                .map(|file_type| file_type.is_dir())
                .unwrap_or(false);
            let child_depth = depth + 1;
            let portable_path = to_portable_path(&child_relative_path);

            entries.push(ProbeEntry {
                relative_path: portable_path.clone(),
                name: name.clone(),
                is_directory,
                depth: child_depth,
            });

            if is_directory
                && child_depth < MAX_SCAN_DEPTH
                && (child_depth <= 3 || should_descend_into(&portable_path, &name))
            {
                queue.push_back((dirent.path(), child_relative_path, child_depth));
            }
        }
    }

    entries
}

fn should_descend_into(relative_path: &str, name: &str) -> bool {
    let lower_name = name.to_lowercase();
    let lower_path = relative_path.to_lowercase();

    if matches!(
        lower_name.as_str(),
        "node_modules" | ".git" | "screenshots" | "captures" | "logs" | "crash reports"
    ) {
        return false;
    }

    lower_name.ends_with("_data")
        || matches!(
            lower_name.as_str(),
            "managed"
                | "plugins"
                | "config"
                | "core"
                | "bepinex"
                | "interop"
                | "binaries"
                | "win64"
                | "content"
                | "paks"
                | "script"
                | "scripts"
                | "~mods"
                | "mods"
                | "ue4ss"
                | "reframework"
                | "autorun"
                | "natives"
        )
        || lower_path.contains("/binaries")
        || lower_path.contains("/content")
}

fn score_map(keys: &[&str]) -> HashMap<String, i32> {
    keys.iter().map(|key| (key.to_string(), 0)).collect()
}

fn add_score(
    scores: &mut HashMap<String, i32>,
    signals: &mut Vec<DetectionSignal>,
    key: &str,
    weight: i32,
    label: &str,
    relative_path: &str,
) {
    *scores.entry(key.to_string()).or_insert(0) += weight;
    signals.push(DetectionSignal {
        label: label.to_string(),
        path: relative_path.to_string(),
        weight,
    });
}

fn choose_highest(scores: &HashMap<String, i32>, fallback: &str) -> String {
    scores
        .iter()
        .max_by_key(|(_, score)| *score)
        .and_then(|(key, score)| if *score > 0 { Some(key.clone()) } else { None })
        .unwrap_or_else(|| fallback.to_string())
}

fn recommend_loader(game_id: Option<&str>, engine: &str, entries: &[ProbeEntry]) -> String {
    if let Some(definition) = game_id.and_then(game_definition_by_id) {
        if let Some(runtime) = definition.bootstrap_runtimes.first() {
            return runtime.clone();
        }
        if !definition.supported_adapters.is_empty() {
            return "none".to_string();
        }
    }

    if game_id.is_none() {
        return "none".to_string();
    }

    if engine == "unreal" && supports_native_script_mods(game_id, entries) {
        return "none".to_string();
    }

    match engine {
        "unity-mono" => "bepinex",
        "unity-il2cpp" => "bepinex-il2cpp",
        "unreal" => "ue4ss",
        "re-engine" => "reframework",
        _ => "none",
    }
    .to_string()
}

fn confidence_for(score: i32) -> f64 {
    (score as f64 / 100.0).clamp(0.0, 0.98)
}

fn format_loader(loader: &str) -> &str {
    match loader {
        "bepinex" => "BepInEx",
        "bepinex-il2cpp" => "BepInEx IL2CPP",
        "ue4ss" => "UE4SS",
        "reframework" => "REFramework",
        "loose-files" => "Loose files",
        _ => "No loader",
    }
}

fn add_installed_mod(store_root: &Path, record: InstalledModRecord) -> Result<(), String> {
    let path = installed_mods_path(store_root);
    let mut store = read_store::<InstalledModRecord>(&path).map_err(error_to_string)?;
    store.items.push(record);
    write_store(&path, &store).map_err(error_to_string)
}

fn write_receipt(
    store_root: &Path,
    profile: &GameProfile,
    record: &InstalledModRecord,
) -> Result<(), String> {
    let receipt_path = profile_dir(store_root, &profile.id)
        .join("receipts")
        .join(format!("{}.json", record.id));
    if let Some(parent) = receipt_path.parent() {
        fs::create_dir_all(parent).map_err(error_to_string)?;
    }
    let content = serde_json::to_string_pretty(record).map_err(error_to_string)?;
    atomic_write(&receipt_path, format!("{}\n", content).as_bytes()).map_err(error_to_string)
}

fn get_profile(store_root: &Path, profile_id: &str) -> Result<GameProfile, String> {
    read_store::<GameProfile>(&profiles_path(store_root))
        .map_err(error_to_string)?
        .items
        .into_iter()
        .find(|profile| profile.id == profile_id)
        .ok_or_else(|| format!("Profile not found: {}", profile_id))
}

fn store_root(app: &AppHandle) -> Result<PathBuf, String> {
    let root = app.path().app_data_dir().map_err(error_to_string)?;
    fs::create_dir_all(root.join("profiles")).map_err(error_to_string)?;
    ensure_store::<GameProfile>(&profiles_path(&root)).map_err(error_to_string)?;
    ensure_store::<InstalledModRecord>(&installed_mods_path(&root)).map_err(error_to_string)?;
    ensure_app_settings(&settings_path(&root)).map_err(error_to_string)?;
    Ok(root)
}

fn ensure_app_settings(path: &Path) -> io::Result<()> {
    if !path.exists() {
        write_app_settings_raw(path, &AppSettings::default())?;
    }
    Ok(())
}

fn read_app_settings(root: &Path) -> Result<AppSettings, String> {
    let path = settings_path(root);
    ensure_app_settings(&path).map_err(error_to_string)?;
    let mut settings = read_json_with_backup::<AppSettings>(&path).map_err(error_to_string)?;

    if !settings.nexus_api_key.trim().is_empty() {
        let legacy_key = settings.nexus_api_key.trim().to_string();
        write_nexus_api_key(Some(&legacy_key))?;
        settings.nexus_api_key.clear();
        settings.nexus_api_key_configured = true;
        write_app_settings(root, &settings)?;
    }

    settings.nexus_api_key = read_nexus_api_key()?.unwrap_or_default();
    settings.nexus_api_key_configured = !settings.nexus_api_key.is_empty();
    Ok(settings)
}

fn write_app_settings(root: &Path, settings: &AppSettings) -> Result<(), String> {
    write_app_settings_raw(&settings_path(root), settings).map_err(error_to_string)
}

fn write_app_settings_raw(path: &Path, settings: &AppSettings) -> io::Result<()> {
    let mut persisted = settings.clone();
    persisted.nexus_api_key.clear();
    let content = serde_json::to_string_pretty(&persisted)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    atomic_write(path, format!("{}\n", content).as_bytes())
}

fn read_nexus_api_key() -> Result<Option<String>, String> {
    let entry =
        keyring::Entry::new(NEXUS_KEYRING_SERVICE, NEXUS_KEYRING_USER).map_err(error_to_string)?;
    match entry.get_password() {
        Ok(value) if !value.trim().is_empty() => Ok(Some(value)),
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(format!(
            "Could not read the Nexus API key from Windows Credential Manager: {error}"
        )),
    }
}

fn write_nexus_api_key(value: Option<&str>) -> Result<(), String> {
    let entry =
        keyring::Entry::new(NEXUS_KEYRING_SERVICE, NEXUS_KEYRING_USER).map_err(error_to_string)?;
    match value {
        Some(value) => entry
            .set_password(value)
            .map_err(|error| format!("Could not save the Nexus API key securely: {error}")),
        None => match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(format!("Could not remove the Nexus API key: {error}")),
        },
    }
}

fn profiles_path(root: &Path) -> PathBuf {
    root.join("profiles.json")
}

fn settings_path(root: &Path) -> PathBuf {
    root.join("settings.json")
}

fn installed_mods_path(root: &Path) -> PathBuf {
    root.join("installed-mods.json")
}

fn pending_nexus_downloads_path(root: &Path) -> PathBuf {
    root.join("pending-nexus-downloads.json")
}

fn profile_dir(root: &Path, profile_id: &str) -> PathBuf {
    root.join("profiles").join(profile_id)
}

fn profile_route_knowledge_path(root: &Path, profile_id: &str) -> PathBuf {
    profile_dir(root, profile_id).join("install-routes.json")
}

fn read_profile_route_knowledge(
    root: &Path,
    profile_id: &str,
) -> Result<Option<ProfileRouteKnowledge>, String> {
    let path = profile_route_knowledge_path(root, profile_id);
    if !path.exists() {
        return Ok(None);
    }
    read_json_with_backup(&path)
        .map(Some)
        .map_err(error_to_string)
}

fn write_profile_route_knowledge(
    root: &Path,
    knowledge: &ProfileRouteKnowledge,
) -> Result<(), String> {
    let path = profile_route_knowledge_path(root, &knowledge.profile_id);
    let content = serde_json::to_string_pretty(knowledge).map_err(error_to_string)?;
    atomic_write(&path, format!("{}\n", content).as_bytes()).map_err(error_to_string)
}

fn profile_backup_dir(root: &Path, profile_id: &str, install_id: &str) -> PathBuf {
    profile_dir(root, profile_id)
        .join("backups")
        .join(install_id)
}

fn profile_package_dir(root: &Path, profile_id: &str, install_id: &str) -> PathBuf {
    profile_dir(root, profile_id)
        .join("packages")
        .join(install_id)
}

fn profile_launch_suspension_dir(root: &Path, profile_id: &str) -> PathBuf {
    profile_dir(root, profile_id).join("launch-suspension")
}

fn profile_launch_suspension_manifest_path(root: &Path, profile_id: &str) -> PathBuf {
    profile_launch_suspension_dir(root, profile_id).join("manifest.json")
}

fn ensure_store<T>(path: &Path) -> io::Result<()>
where
    T: Serialize + DeserializeOwned,
{
    let _guard = lock_store_io()?;
    ensure_store_unlocked::<T>(path)
}

fn ensure_store_unlocked<T>(path: &Path) -> io::Result<()>
where
    T: Serialize + DeserializeOwned,
{
    if !path.exists() {
        let store = StoreFile::<T> {
            version: 1,
            items: Vec::new(),
        };
        let content = serde_json::to_string_pretty(&store)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        atomic_write_unlocked(path, format!("{}\n", content).as_bytes())?;
    }
    Ok(())
}

fn read_store<T>(path: &Path) -> io::Result<StoreFile<T>>
where
    T: Serialize + DeserializeOwned,
{
    let _guard = lock_store_io()?;
    ensure_store_unlocked::<T>(path)?;
    read_json_with_backup_unlocked(path)
}

fn write_store<T>(path: &Path, store: &StoreFile<T>) -> io::Result<()>
where
    T: Serialize + DeserializeOwned,
{
    let content = serde_json::to_string_pretty(store)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    atomic_write(path, format!("{}\n", content).as_bytes())
}

fn lock_mutations() -> Result<MutexGuard<'static, ()>, String> {
    MUTATION_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "UniLoader's operation queue was poisoned by an earlier failure.".to_string())
}

fn read_json_with_backup<T>(path: &Path) -> io::Result<T>
where
    T: DeserializeOwned,
{
    let _guard = lock_store_io()?;
    read_json_with_backup_unlocked(path)
}

fn read_json_with_backup_unlocked<T>(path: &Path) -> io::Result<T>
where
    T: DeserializeOwned,
{
    let primary = fs::read_to_string(path)
        .and_then(|raw| parse_json_allow_bom(&raw).map_err(invalid_data_error));
    if primary.is_ok() {
        return primary;
    }

    let backup_path = backup_path_for(path);
    let backup_raw = fs::read_to_string(&backup_path)?;
    let recovered = parse_json_allow_bom::<T>(&backup_raw).map_err(invalid_data_error)?;
    atomic_write_unlocked_with_backup(path, backup_raw.as_bytes(), false)?;
    Ok(recovered)
}

fn invalid_data_error(error: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

fn backup_path_for(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("store.json");
    path.with_file_name(format!("{file_name}.bak"))
}

fn atomic_write(path: &Path, content: &[u8]) -> io::Result<()> {
    let _guard = lock_store_io()?;
    atomic_write_unlocked(path, content)
}

fn atomic_write_unlocked(path: &Path, content: &[u8]) -> io::Result<()> {
    atomic_write_unlocked_with_backup(path, content, true)
}

fn lock_store_io() -> io::Result<MutexGuard<'static, ()>> {
    STORE_WRITE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| io::Error::other("UniLoader's storage lock was poisoned"))
}

fn atomic_write_unlocked_with_backup(
    path: &Path,
    content: &[u8],
    preserve_previous: bool,
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("store.json");
    let temporary_path = path.with_file_name(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
    let backup_path = backup_path_for(path);

    let write_result = (|| -> io::Result<()> {
        let mut temporary = File::create(&temporary_path)?;
        temporary.write_all(content)?;
        temporary.sync_all()?;

        if path.exists() && preserve_previous {
            fs::copy(path, &backup_path)?;
        }
        if path.exists() {
            fs::remove_file(path)?;
        }

        if let Err(error) = fs::rename(&temporary_path, path) {
            if !path.exists() && backup_path.exists() {
                let _ = fs::copy(&backup_path, path);
            }
            return Err(error);
        }
        Ok(())
    })();

    if temporary_path.exists() {
        let _ = fs::remove_file(&temporary_path);
    }
    write_result
}

fn parse_json_allow_bom<T>(raw: &str) -> serde_json::Result<T>
where
    T: DeserializeOwned,
{
    serde_json::from_str(raw.trim_start_matches('\u{feff}'))
}

fn safe_join(root: &Path, relative_path: &str) -> Result<PathBuf, String> {
    let normalized = normalize_archive_path(relative_path);
    let relative = Path::new(&normalized);
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!("Unsafe archive path: {}", relative_path));
    }

    Ok(root.join(relative))
}

fn to_portable_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn now_string() -> String {
    Utc::now().to_rfc3339()
}

fn error_to_string(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn is_newer_version(candidate: &str, current: &str) -> bool {
    let candidate_parts = version_parts(candidate);
    let current_parts = version_parts(current);
    let max_len = candidate_parts.len().max(current_parts.len()).max(3);

    for index in 0..max_len {
        let candidate_part = *candidate_parts.get(index).unwrap_or(&0);
        let current_part = *current_parts.get(index).unwrap_or(&0);
        if candidate_part != current_part {
            return candidate_part > current_part;
        }
    }

    false
}

fn version_parts(version: &str) -> Vec<u64> {
    version
        .trim()
        .trim_start_matches('v')
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<u64>().ok())
        .collect()
}

fn select_update_installer_asset(release: &GithubReleaseResponse) -> Option<&GithubReleaseAsset> {
    release
        .assets
        .iter()
        .filter_map(|asset| update_installer_asset_score(&asset.name).map(|score| (score, asset)))
        .max_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| left.1.name.cmp(&right.1.name))
        })
        .map(|(_, asset)| asset)
}

fn update_installer_asset_score(name: &str) -> Option<i32> {
    let lower_name = name.to_lowercase();
    if !lower_name.starts_with("uniloader") {
        return None;
    }

    let mut score = if lower_name.ends_with("_x64-setup.exe")
        || lower_name.ends_with("-x64-setup.exe")
        || lower_name.ends_with("-setup.exe")
    {
        100
    } else if lower_name.ends_with(".msi") {
        70
    } else {
        return None;
    };

    if lower_name.contains("x64") {
        score += 8;
    }
    if lower_name.contains("setup") {
        score += 6;
    }

    Some(score)
}

fn open_folder_in_shell(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let explorer_target =
            PathBuf::from(normalize_profile_game_path(path.to_string_lossy().as_ref()));
        if !explorer_target.is_dir() {
            return Err(format!(
                "Folder no longer exists: {}",
                explorer_target.to_string_lossy()
            ));
        }
        Command::new("explorer.exe")
            .arg(explorer_target)
            .spawn()
            .map(|_| ())
            .map_err(error_to_string)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        Command::new(opener)
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(error_to_string)
    }
}

fn open_url_in_shell(url: &str) -> Result<(), String> {
    let trimmed_url = validated_shell_url(url)?;

    #[cfg(target_os = "windows")]
    {
        Command::new("rundll32")
            .arg("url.dll,FileProtocolHandler")
            .arg(trimmed_url)
            .spawn()
            .map(|_| ())
            .map_err(error_to_string)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        Command::new(opener)
            .arg(trimmed_url)
            .spawn()
            .map(|_| ())
            .map_err(error_to_string)
    }
}

fn validated_shell_url(url: &str) -> Result<String, String> {
    let trimmed_url = url.trim();
    if trimmed_url.chars().any(char::is_control) {
        return Err("Link contains invalid control characters.".to_string());
    }
    if trimmed_url.starts_with("steam://") {
        return Ok(trimmed_url.to_string());
    }
    validate_https_url(trimmed_url)?;
    Ok(trimmed_url.to_string())
}

fn validated_update_url(value: &str) -> Result<String, String> {
    let parsed = validate_https_url(value.trim())?;
    if parsed.host_str() != Some("github.com") {
        return Err("Updates can only be downloaded from GitHub releases.".to_string());
    }
    let expected_prefix = format!("/{APP_UPDATE_REPOSITORY}/releases/download/");
    if !parsed.path().starts_with(&expected_prefix) {
        return Err("The update URL is not an official UniLoader release asset.".to_string());
    }
    Ok(parsed.to_string())
}

fn download_update_checksum(client: &Client, checksum_url: &str) -> Result<String, String> {
    validated_update_url(checksum_url.trim_end_matches(".sha256"))?;
    let mut response = client
        .get(checksum_url)
        .send()
        .map_err(error_to_string)?
        .error_for_status()
        .map_err(|error| {
            format!("The release is missing its required SHA-256 checksum: {error}")
        })?;
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(4097)
        .read_to_end(&mut bytes)
        .map_err(error_to_string)?;
    if bytes.len() > 4096 {
        return Err("The update checksum file is unexpectedly large.".to_string());
    }
    let text = String::from_utf8(bytes).map_err(error_to_string)?;
    let hash = text
        .split_whitespace()
        .next()
        .filter(|value| value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit()))
        .ok_or_else(|| "The update checksum file is invalid.".to_string())?;
    Ok(hash.to_lowercase())
}

fn update_download_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    {
        if let Some(user_profile) = std::env::var_os("USERPROFILE") {
            return Ok(PathBuf::from(user_profile).join("Downloads"));
        }
    }

    if let Some(home) = std::env::var_os("HOME") {
        return Ok(PathBuf::from(home).join("Downloads"));
    }

    Ok(std::env::temp_dir())
}

fn update_installer_file_name(url: &str, file_name: Option<&str>) -> Result<String, String> {
    let raw_name = file_name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .or_else(|| {
            url.rsplit('/')
                .next()
                .and_then(|part| part.split('?').next())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "UniLoader-update-setup.exe".to_string());
    let safe_name = sanitize_file_segment(&raw_name);
    let lower_name = safe_name.to_lowercase();

    if !(lower_name.ends_with(".exe") || lower_name.ends_with(".msi")) {
        return Err("Update asset must be a Windows .exe or .msi installer.".to_string());
    }

    Ok(safe_name)
}

fn launch_update_installer(path: &Path) -> Result<(), String> {
    if !path.is_file() {
        return Err("Downloaded installer was not found.".to_string());
    }

    Command::new(path)
        .spawn()
        .map(|_| ())
        .map_err(error_to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_paths_must_be_inside_the_exact_game_directory() {
        let game_root = Path::new(r"D:\Steam\steamapps\common\Windrose");

        assert!(path_is_within_directory(
            Path::new(r"D:\Steam\steamapps\common\Windrose\R5\Binaries\Windrose.exe"),
            game_root,
        ));
        assert!(path_is_within_directory(
            Path::new(r"\\?\D:\Steam\steamapps\common\Windrose\Windrose.exe"),
            game_root,
        ));
        assert!(!path_is_within_directory(
            Path::new(r"D:\Steam\steamapps\common\Windrose Server\Windrose.exe"),
            game_root,
        ));
        assert!(!path_is_within_directory(
            Path::new(r"D:\Tools\Windrose.exe"),
            game_root,
        ));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_process_path_probe_reads_current_executable() {
        let expected = fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
        let actual = process_executable_path(std::process::id())
            .expect("the current process path should be queryable");

        assert_eq!(
            normalize_process_path(&actual),
            normalize_process_path(&expected)
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "requires an interactive desktop process namespace"]
    fn running_game_probe_tracks_a_process_inside_the_game_directory() {
        use std::os::windows::process::CommandExt;
        use std::process::Stdio;
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

        let game_root = temp_game_dir("running-game-probe");
        fs::create_dir_all(&game_root).unwrap();
        let probe_executable = game_root.join("UniLoaderGameProbe.exe");
        fs::copy(std::env::current_exe().unwrap(), &probe_executable).unwrap();

        let mut child = Command::new(&probe_executable)
            .args([
                "--exact",
                "tests::running_game_probe_child_process",
                "--nocapture",
            ])
            .env("UNILOADER_PROCESS_PROBE_CHILD", "1")
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();

        let detected = (0..20).any(|_| {
            std::thread::sleep(Duration::from_millis(50));
            game_process_running(&game_root).unwrap()
        });
        assert!(
            detected,
            "the process inside the game folder was not detected"
        );

        child.kill().unwrap();
        child.wait().unwrap();
        let stopped = (0..20).any(|_| {
            if !game_process_running(&game_root).unwrap() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
            false
        });
        assert!(stopped, "the stopped game process remained visible");

        let _ = fs::remove_dir_all(game_root);
    }

    #[test]
    fn running_game_probe_child_process() {
        if std::env::var_os("UNILOADER_PROCESS_PROBE_CHILD").is_some() {
            std::thread::sleep(Duration::from_secs(10));
        }
    }

    fn online_mod_fixture(name: &str, description: &str) -> OnlineModRecord {
        OnlineModRecord {
            id: format!("test:{}", sanitize_file_segment(name)),
            provider: "thunderstore".to_string(),
            provider_label: "Thunderstore".to_string(),
            game_id: Some("test-game".to_string()),
            provider_game_id: Some("test-game".to_string()),
            name: name.to_string(),
            owner: "Test Author".to_string(),
            version: "1.0.0".to_string(),
            description: description.to_string(),
            categories: Vec::new(),
            downloads: 100,
            rating_score: 0,
            dependency_count: 0,
            file_size: None,
            icon_url: None,
            package_url: None,
            website_url: None,
            installed: false,
            created_at: None,
            updated_at: None,
            install_supported: true,
            install_note: None,
        }
    }

    #[test]
    fn download_requests_retry_transient_server_errors() {
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for status_line in ["502 Bad Gateway", "200 OK"] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 1024];
                let _ = stream.read(&mut request).unwrap();
                write!(
                    stream,
                    "HTTP/1.1 {status_line}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
                stream.flush().unwrap();
            }
        });

        let client = Client::builder().build().unwrap();
        let response = request_download_with_retry(&client, &format!("http://{address}/mod.zip"))
            .expect("a transient provider failure should be retried");

        assert_eq!(response.status(), StatusCode::OK);
        server.join().unwrap();
    }

    #[test]
    fn download_retry_statuses_exclude_permanent_client_errors() {
        assert!(is_transient_download_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_transient_download_status(StatusCode::BAD_GATEWAY));
        assert!(is_transient_download_status(
            StatusCode::SERVICE_UNAVAILABLE
        ));
        assert!(!is_transient_download_status(StatusCode::BAD_REQUEST));
        assert!(!is_transient_download_status(StatusCode::FORBIDDEN));
        assert!(!is_transient_download_status(StatusCode::NOT_FOUND));
    }

    #[test]
    fn downloaded_archive_validation_accepts_archives_and_rejects_web_payloads() {
        let root = temp_game_dir("downloaded-archive-validation");
        fs::create_dir_all(&root).unwrap();

        let valid_zip = root.join("valid.download");
        let output = File::create(&valid_zip).unwrap();
        let mut zip = ZipWriter::new(output);
        add_bytes_to_zip(&mut zip, "plugin/mod.dll", b"mod payload").unwrap();
        zip.finish().unwrap();
        assert!(validate_downloaded_archive(&valid_zip, "zip").is_ok());

        let fake_zip = root.join("provider-message.download");
        fs::write(&fake_zip, b"<html>download authorization expired</html>").unwrap();
        let error = validate_downloaded_archive(&fake_zip, "zip").unwrap_err();
        assert!(error.contains("invalid ZIP archive"));

        let valid_7z = root.join("valid-7z.download");
        fs::write(&valid_7z, [0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C, 0, 0]).unwrap();
        assert!(validate_downloaded_archive(&valid_7z, "7z").is_ok());

        let valid_rar = root.join("valid-rar.download");
        fs::write(&valid_rar, [0x52, 0x61, 0x72, 0x21, 0x1A, 0x07, 0x01, 0x00]).unwrap();
        assert!(validate_downloaded_archive(&valid_rar, "rar").is_ok());

        let _ = fs::remove_dir_all(root);
    }

    fn assert_supported_import_deploys(
        root: &Path,
        import_path: &Path,
        prepared_as_directory: bool,
    ) {
        let store_root = root.join("store");
        let game_root = root.join("game");
        fs::create_dir_all(game_root.join("R5/Content/Paks/~mods")).unwrap();
        let scanned = scan_import_source(&store_root, import_path).unwrap();
        assert_eq!(
            Path::new(&scanned.archive_path).is_dir(),
            prepared_as_directory
        );

        let mut profile = test_profile("windrose", "Windrose", "unreal", "ue4ss");
        profile.game_path = game_root.to_string_lossy().to_string();
        let analysis = analyze_scanned_archive(scanned, &profile);
        let plan = analysis.recommended_plan.clone().unwrap();
        let result = install_archive_impl(
            &store_root,
            &profile,
            import_path.to_string_lossy().as_ref(),
            import_path.file_name().and_then(|name| name.to_str()),
            Some(analysis.package_identity),
            &plan,
        )
        .unwrap();

        assert!(!result.files_written.is_empty());
        assert!(game_root
            .join("R5/Content/Paks/~mods/FutureMod_P.pak")
            .is_file());
        assert_eq!(
            Path::new(&result.archive_path).is_dir(),
            prepared_as_directory
        );
    }

    #[test]
    fn zip_imports_deploy_from_their_archive_source() {
        let root = temp_game_dir("zip-install-source");
        fs::create_dir_all(&root).unwrap();
        let archive_path = root.join("FutureMod.zip");
        let output = File::create(&archive_path).unwrap();
        let mut zip = ZipWriter::new(output);
        add_bytes_to_zip(&mut zip, "FutureMod_P.pak", b"future mod").unwrap();
        zip.finish().unwrap();

        assert_supported_import_deploys(&root, &archive_path, false);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn seven_zip_imports_deploy_from_their_extracted_source() {
        let root = temp_game_dir("seven-zip-install-source");
        let source_root = root.join("source");
        touch(&source_root, "FutureMod_P.pak");
        let archive_path = root.join("FutureMod.7z");
        sevenz_rust2::compress_to_path(&source_root, &archive_path).unwrap();

        assert_supported_import_deploys(&root, &archive_path, true);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rar_imports_deploy_from_their_extracted_source() {
        let root = temp_game_dir("rar-install-source");
        fs::create_dir_all(&root).unwrap();
        let archive_path = root.join("FutureMod.rar");
        let entries = [rars::rar50::StoredEntry {
            name: b"FutureMod_P.pak",
            data: b"future mod",
            mtime: None,
            attributes: 0x20,
            host_os: 3,
        }];
        let archive = rars::rar50::Rar50Writer::new(rars::rar50::WriterOptions::default())
            .stored_entries(&entries)
            .finish()
            .unwrap();
        fs::write(&archive_path, archive).unwrap();

        assert_supported_import_deploys(&root, &archive_path, true);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn folder_imports_deploy_from_their_directory_source() {
        let root = temp_game_dir("folder-install-source");
        let source_root = root.join("FutureMod");
        touch(&source_root, "FutureMod_P.pak");

        assert_supported_import_deploys(&root, &source_root, true);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn steam_app_id_is_inferred_from_the_adjacent_manifest() {
        let library = temp_game_dir("steam-profile-artwork");
        let steamapps = library.join("steamapps");
        let game_path = steamapps.join("common").join("Windrose");
        fs::create_dir_all(&game_path).unwrap();
        fs::write(
            steamapps.join("appmanifest_3041230.acf"),
            r#""AppState"
{
    "appid"        "3041230"
    "name"         "Windrose"
    "installdir"   "Windrose"
}"#,
        )
        .unwrap();

        assert_eq!(
            infer_steam_app_id_for_game_path(&game_path).as_deref(),
            Some("3041230")
        );
        assert!(infer_steam_app_id_for_game_path(&library.join("StandaloneGame")).is_none());

        let _ = fs::remove_dir_all(library);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn profile_game_paths_use_native_windows_separators() {
        assert_eq!(
            normalize_profile_game_path("d:/steam\\steamapps/common/Windrose"),
            "d:\\steam\\steamapps\\common\\Windrose"
        );
        assert_eq!(
            normalize_profile_game_path("//server/share/Games/Windrose"),
            "\\\\server\\share\\Games\\Windrose"
        );
    }

    #[test]
    fn selected_nexus_file_is_used_instead_of_the_recommended_default() {
        let files = vec![
            NexusFileRecord {
                file_id: 10,
                name: "2x minerals".to_string(),
                version: Some("2x".to_string()),
                category_name: Some("MAIN".to_string()),
                is_primary: Some(true),
                uploaded_timestamp: Some(10),
                uploaded_time: None,
                file_name: Some("minerals-2x.zip".to_string()),
                size: Some(100),
                size_kb: None,
                description: None,
            },
            NexusFileRecord {
                file_id: 20,
                name: "10x minerals".to_string(),
                version: Some("10x".to_string()),
                category_name: Some("OPTIONAL".to_string()),
                is_primary: Some(false),
                uploaded_timestamp: Some(20),
                uploaded_time: None,
                file_name: Some("minerals-10x.7z".to_string()),
                size: Some(200),
                size_kb: None,
                description: None,
            },
        ];

        assert_eq!(choose_nexus_file(&files).unwrap().file_id, 10);
        assert_eq!(
            choose_requested_nexus_file(&files, Some("20"))
                .unwrap()
                .unwrap()
                .file_id,
            20
        );
        assert!(choose_requested_nexus_file(&files, Some("999")).is_err());
    }

    #[test]
    fn nexus_manager_download_page_requests_an_nxm_handoff() {
        assert_eq!(
            nexus_manager_download_page_url("stardewvalley", 49335, 175386),
            "https://www.nexusmods.com/stardewvalley/mods/49335?tab=files&file_id=175386&nmm=1"
        );
    }

    #[test]
    fn signed_nxm_links_are_parsed_without_trusting_extra_query_fields() {
        let expires = Utc::now().timestamp() + 600;
        let parsed = parse_nexus_nxm_link(&format!(
            "nxm://stardewvalley/mods/49335/files/175386?key=abc_DEF-123&expires={expires}&user_id=42&campaign=test"
        ))
        .unwrap();

        assert_eq!(parsed.domain, "stardewvalley");
        assert_eq!(parsed.mod_id, 49335);
        assert_eq!(parsed.file_id, 175386);
        assert_eq!(parsed.key, "abc_DEF-123");
        assert_eq!(parsed.expires, expires);
        assert_eq!(parsed.user_id, 42);
    }

    #[test]
    fn malformed_or_expired_nxm_links_are_rejected() {
        let future = Utc::now().timestamp() + 600;
        let expired = Utc::now().timestamp() - 60;

        assert!(parse_nexus_nxm_link(&format!(
            "https://stardewvalley/mods/1/files/2?key=test&expires={future}&user_id=42"
        ))
        .is_err());
        assert!(parse_nexus_nxm_link(&format!(
            "nxm://stardewvalley/mods/1/files/2?expires={future}&user_id=42"
        ))
        .is_err());
        assert!(parse_nexus_nxm_link(&format!(
            "nxm://stardewvalley/mods/1/files/2?key=test&expires={expired}&user_id=42"
        ))
        .is_err());
    }

    #[test]
    fn thunderstore_dependency_versions_keep_their_dotted_version_path() {
        let package_ref = ThunderstorePackageRef {
            namespace: "Zehs".to_string(),
            name: "REPOLib".to_string(),
            version: Some("3.0.0".to_string()),
        };

        assert_eq!(
            thunderstore_package_version_url(&package_ref, "3.0.0"),
            "https://thunderstore.io/api/experimental/package/Zehs/REPOLib/3.0.0"
        );
    }

    #[test]
    fn discovery_hides_external_managers_but_keeps_in_game_management_mods() {
        assert!(is_external_mod_manager_listing(&online_mod_fixture(
            "Kesomannen Gale Mod Manager",
            "A modern and lightweight alternative mod manager for Thunderstore."
        )));
        assert!(is_external_mod_manager_listing(&online_mod_fixture(
            "unreal shimloader",
            "Thunderstore Mod Manager and r2modmanPlus support for RE-UE4SS."
        )));
        assert!(is_external_mod_manager_listing(&online_mod_fixture(
            "Vortex Extension",
            "Adds support for this game to Vortex."
        )));

        assert!(!is_external_mod_manager_listing(&online_mod_fixture(
            "BepInEx Configuration Manager",
            "An in-game configuration manager for installed plugins."
        )));
        assert!(!is_external_mod_manager_listing(&online_mod_fixture(
            "Server Mod Browser",
            "Browse and inspect server-side mods from inside the game."
        )));
    }

    #[test]
    fn discovery_search_matches_only_the_mod_name() {
        let title_match = online_mod_fixture(
            "Increase Drop Rates",
            "Adjusts resource rewards throughout the game.",
        );
        let detail_only_match = online_mod_fixture(
            "More Animal Resources",
            "Includes configurable drop rate controls.",
        );

        assert!(online_mod_matches_query(&title_match, "drop rate"));
        assert!(!online_mod_matches_query(&detail_only_match, "drop rate"));
    }

    #[test]
    fn clean_launch_preserves_individual_mod_choices() {
        let store_root = temp_game_dir("clean-launch-store");
        let game_root = store_root.join("game");
        fs::create_dir_all(game_root.join("Mods")).unwrap();
        let enabled_file = game_root.join("Mods/enabled.dll");
        let disabled_file = game_root.join("Mods/disabled.dll");
        let runtime_file = game_root.join("runtime.dll");
        fs::write(&enabled_file, b"enabled mod").unwrap();
        fs::write(&disabled_file, b"disabled sentinel").unwrap();
        fs::write(&runtime_file, b"runtime").unwrap();

        let profile = GameProfile {
            id: "profile-clean-launch".to_string(),
            name: "Test Game".to_string(),
            game_path: game_root.to_string_lossy().to_string(),
            game_id: Some("test-game".to_string()),
            steam_app_id: Some("123".to_string()),
            engine: "unity-mono".to_string(),
            loader: "bepinex".to_string(),
            created_at: now_string(),
            updated_at: now_string(),
        };
        let record =
            |id: &str, file: &Path, enabled: bool, runtime_id: Option<&str>| InstalledModRecord {
                id: id.to_string(),
                profile_id: profile.id.clone(),
                archive_path: store_root
                    .join(format!("missing-{id}.zip"))
                    .to_string_lossy()
                    .to_string(),
                archive_name: format!("{id}.zip"),
                display_name: Some(id.to_string()),
                package_id: None,
                dependency_string: None,
                icon_url: None,
                adapter_id: "test-adapter".to_string(),
                summary: String::new(),
                installed_at: now_string(),
                files_written: vec![file.to_string_lossy().to_string()],
                backups_written: Vec::new(),
                written_file_hashes: HashMap::new(),
                dependencies: Vec::new(),
                config_files: Vec::new(),
                runtime_id: runtime_id.map(str::to_string),
                externally_managed: true,
                enabled,
                last_status: (if enabled { "installed" } else { "disabled" }).to_string(),
                plan: None,
            };
        write_store(
            &installed_mods_path(&store_root),
            &StoreFile {
                version: 1,
                items: vec![
                    record("enabled-mod", &enabled_file, true, None),
                    record("disabled-mod", &disabled_file, false, None),
                    record("runtime", &runtime_file, true, Some("bepinex")),
                ],
            },
        )
        .unwrap();

        assert_eq!(
            prepare_profile_mod_launch(&store_root, &profile, false).unwrap(),
            1
        );
        assert!(!enabled_file.exists());
        assert!(disabled_file.exists());
        assert!(runtime_file.exists());

        let suspended_store =
            read_store::<InstalledModRecord>(&installed_mods_path(&store_root)).unwrap();
        assert!(suspended_store.items[0].enabled);
        assert!(!suspended_store.items[1].enabled);
        assert!(suspended_store.items[2].enabled);

        let suspension = read_profile_launch_suspension(&store_root, &profile.id).unwrap();
        let suspended_health = mod_file_health_for_record(&suspended_store.items[0], &suspension);
        assert!(suspended_health.missing_files.is_empty());
        assert_eq!(
            suspended_health.suspended_files,
            vec![enabled_file.to_string_lossy().to_string()]
        );

        let genuinely_missing_file = game_root.join("Mods/genuinely-missing.dll");
        let genuinely_missing_record =
            record("genuinely-missing", &genuinely_missing_file, true, None);
        let genuinely_missing_health =
            mod_file_health_for_record(&genuinely_missing_record, &suspension);
        assert_eq!(
            genuinely_missing_health.missing_files,
            vec![genuinely_missing_file.to_string_lossy().to_string()]
        );
        assert!(genuinely_missing_health.suspended_files.is_empty());

        assert_eq!(
            prepare_profile_mod_launch(&store_root, &profile, true).unwrap(),
            1
        );
        assert_eq!(fs::read(&enabled_file).unwrap(), b"enabled mod");
        assert!(disabled_file.exists());
        assert!(runtime_file.exists());
        assert!(!profile_launch_suspension_dir(&store_root, &profile.id).exists());

        let _ = fs::remove_dir_all(store_root);
    }

    #[test]
    fn nexus_discovery_splits_later_pages_into_safe_query_batches() {
        assert_eq!(nexus_discovery_batch_size(20), 20);
        assert_eq!(nexus_discovery_batch_size(40), 40);
        assert_eq!(nexus_discovery_batch_size(60), 40);
        assert_eq!(nexus_discovery_batch_size(360), 40);
    }

    #[test]
    fn update_version_comparison_handles_multi_digit_segments() {
        assert!(is_newer_version("0.10.0", "0.2.0"));
        assert!(is_newer_version("v1.0.0", "0.9.9"));
        assert!(!is_newer_version("0.1.0", "0.1.0"));
        assert!(!is_newer_version("0.1.0", "0.2.0"));
    }

    #[test]
    fn corrupted_json_store_recovers_without_destroying_its_backup() {
        let root = temp_game_dir("store-recovery");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("records.json");
        let first = StoreFile::<DependencySpec> {
            version: 1,
            items: Vec::new(),
        };
        let second = StoreFile {
            version: 1,
            items: vec![DependencySpec {
                id: "runtime:test".to_string(),
                name: "Test Runtime".to_string(),
                version: Some("1.0".to_string()),
                provider: "manual".to_string(),
                required: true,
                status: "missing".to_string(),
                source: None,
                notes: None,
            }],
        };

        write_store(&path, &first).unwrap();
        write_store(&path, &second).unwrap();
        fs::write(&path, "not json").unwrap();

        let recovered = read_store::<DependencySpec>(&path).unwrap();
        assert!(recovered.items.is_empty());
        let backup = fs::read_to_string(backup_path_for(&path)).unwrap();
        let backup_store = parse_json_allow_bom::<StoreFile<DependencySpec>>(&backup).unwrap();
        assert!(backup_store.items.is_empty());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn store_reads_wait_for_atomic_replacements() {
        let root = temp_game_dir("store-read-lock");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("records.json");
        write_store(
            &path,
            &StoreFile {
                version: 1,
                items: vec![DependencySpec {
                    id: "runtime:test".to_string(),
                    name: "Test Runtime".to_string(),
                    version: None,
                    provider: "test".to_string(),
                    required: true,
                    status: "installed".to_string(),
                    source: None,
                    notes: None,
                }],
            },
        )
        .unwrap();

        let guard = lock_store_io().unwrap();
        let reader_path = path.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            sender
                .send(read_store::<DependencySpec>(&reader_path))
                .unwrap();
        });

        assert!(receiver
            .recv_timeout(std::time::Duration::from_millis(40))
            .is_err());
        drop(guard);
        let store = receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap()
            .unwrap();
        reader.join().unwrap();
        assert_eq!(store.items.len(), 1);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn update_checker_prefers_setup_exe_assets() {
        let release = GithubReleaseResponse {
            tag_name: "v0.5".to_string(),
            html_url: Some("https://github.com/Chucksterboy/UniLoader/releases/tag/v0.5".to_string()),
            assets: vec![
                GithubReleaseAsset {
                    name: "UniLoader_0.5.0_x64_en-US.msi".to_string(),
                    browser_download_url:
                        "https://github.com/Chucksterboy/UniLoader/releases/download/v0.5/UniLoader_0.5.0_x64_en-US.msi"
                            .to_string(),
                },
                GithubReleaseAsset {
                    name: "UniLoader_0.5.0_x64-setup.exe".to_string(),
                    browser_download_url:
                        "https://github.com/Chucksterboy/UniLoader/releases/download/v0.5/UniLoader_0.5.0_x64-setup.exe"
                            .to_string(),
                },
            ],
        };

        let asset = select_update_installer_asset(&release).unwrap();
        assert_eq!(asset.name, "UniLoader_0.5.0_x64-setup.exe");
    }

    #[test]
    fn provider_candidates_cover_slugs_and_known_aliases() {
        let windrose = GameProfile {
            id: "profile-1".to_string(),
            name: "My Friends Server".to_string(),
            game_path: "D:/Steam/steamapps/common/Windrose".to_string(),
            game_id: Some("windrose".to_string()),
            steam_app_id: None,
            engine: "unreal".to_string(),
            loader: "ue4ss".to_string(),
            created_at: now_string(),
            updated_at: now_string(),
        };
        let windrose_candidates = provider_slug_candidates(&windrose);
        assert!(windrose_candidates.contains(&"windrose".to_string()));
        assert!(!windrose_candidates.contains(&"myfriendsserver".to_string()));

        let dragonwilds = GameProfile {
            id: "profile-2".to_string(),
            name: "Coop Pack".to_string(),
            game_path: "D:/Steam/steamapps/common/DragonWilds".to_string(),
            game_id: None,
            steam_app_id: None,
            engine: "unreal".to_string(),
            loader: "ue4ss".to_string(),
            created_at: now_string(),
            updated_at: now_string(),
        };
        let dragonwilds_candidates = provider_slug_candidates(&dragonwilds);
        assert!(dragonwilds_candidates.contains(&"dragonwilds".to_string()));
        assert!(dragonwilds_candidates.contains(&"dragon-wilds".to_string()));
        assert!(dragonwilds_candidates.contains(&"runescapedragonwilds".to_string()));
        assert!(
            provider_name_candidates(&dragonwilds).contains(&"RuneScape: Dragonwilds".to_string())
        );
        let lookup_names = nexus_game_lookup_names(&dragonwilds, "runescapedragonwilds");
        assert_eq!(lookup_names.first().unwrap(), "RuneScape: Dragonwilds");
    }

    #[test]
    fn nexus_text_decodes_numeric_entities_for_display_and_route_scanning() {
        let text = "Extract to RSDragonwilds&#92;Binaries&#x5c;Win64 &amp; restart";
        assert_eq!(
            clean_nexus_summary(Some(text.to_string())),
            "Extract to RSDragonwilds\\Binaries\\Win64 & restart"
        );
        assert_eq!(
            provider_text_for_route_scan(text),
            "Extract to RSDragonwilds\\Binaries\\Win64 & restart"
        );
    }

    #[test]
    #[ignore = "live provider smoke test"]
    fn live_nexus_game_id_lookup_resolves_compact_domains_using_provider_identity() {
        let profile = GameProfile {
            id: "profile-dragonwilds".to_string(),
            name: "Coop Pack".to_string(),
            game_path: "D:/Steam/steamapps/common/RSDragonwilds".to_string(),
            game_id: None,
            steam_app_id: Some("1374490".to_string()),
            engine: "unreal".to_string(),
            loader: "ue4ss".to_string(),
            created_at: now_string(),
            updated_at: now_string(),
        };
        let client = provider_client().unwrap();

        assert_eq!(
            fetch_nexus_game_id_for_domain(&client, &profile, "runescapedragonwilds", None,)
                .unwrap(),
            7597
        );
    }

    #[test]
    fn incompatible_install_plan_blocks_unreal_mods_on_unity_profiles() {
        let profile = GameProfile {
            id: "profile-1".to_string(),
            name: "Valheim".to_string(),
            game_path: "C:/Games/Valheim".to_string(),
            game_id: Some("valheim".to_string()),
            steam_app_id: None,
            engine: "unity-mono".to_string(),
            loader: "bepinex".to_string(),
            created_at: now_string(),
            updated_at: now_string(),
        };
        let plan = InstallPlan {
            adapter_id: "unreal-pak".to_string(),
            adapter_name: "Unreal Pak Files".to_string(),
            confidence: 0.66,
            summary: "Deploy one pak file.".to_string(),
            mappings: Vec::new(),
            dependencies: Vec::new(),
            warnings: Vec::new(),
            requires_confirmation: false,
        };

        let reason = incompatible_install_plan_reason(&profile, &plan).unwrap();
        assert!(reason.contains("Unreal Engine games"));
        assert!(reason.contains("Unity Mono"));
    }

    #[test]
    fn duplicate_mod_detection_blocks_same_clean_name_and_adapter() {
        let root = temp_game_dir("duplicate-mod-detection");
        let profile = GameProfile {
            id: "profile-1".to_string(),
            name: "Windrose".to_string(),
            game_path: "C:/Games/Windrose".to_string(),
            game_id: None,
            steam_app_id: None,
            engine: "unreal".to_string(),
            loader: "ue4ss".to_string(),
            created_at: now_string(),
            updated_at: now_string(),
        };
        let record = InstalledModRecord {
            id: "mod-1".to_string(),
            profile_id: profile.id.clone(),
            archive_path: "C:/UniLoader/packages/ShipLootx2.zip".to_string(),
            archive_name: "ShipLootx2-172-3-1777184245.zip".to_string(),
            display_name: Some("Ship Lootx 2".to_string()),
            package_id: None,
            dependency_string: None,
            icon_url: None,
            adapter_id: "unreal-pak".to_string(),
            summary: String::new(),
            installed_at: now_string(),
            files_written: Vec::new(),
            backups_written: Vec::new(),
            written_file_hashes: HashMap::new(),
            dependencies: Vec::new(),
            config_files: Vec::new(),
            runtime_id: None,
            externally_managed: false,
            enabled: true,
            last_status: "installed".to_string(),
            plan: None,
        };
        write_store(
            &installed_mods_path(&root),
            &StoreFile {
                version: 1,
                items: vec![record],
            },
        )
        .unwrap();
        let plan = InstallPlan {
            adapter_id: "unreal-pak".to_string(),
            adapter_name: "Unreal Pak Files".to_string(),
            confidence: 0.88,
            summary: "Deploy one pak file.".to_string(),
            mappings: vec![mapping(
                "ShipLootx2.pak",
                "game",
                "Content/Paks/~mods/ShipLootx2.pak",
                "Test pak.",
            )],
            dependencies: Vec::new(),
            warnings: Vec::new(),
            requires_confirmation: false,
        };
        let metadata = InstallMetadata {
            archive_name: Some("ShipLootx2-172-3-1777184245.zip".to_string()),
            ..InstallMetadata::default()
        };

        let reason = duplicate_installed_mod_reason(
            &root,
            &profile,
            &plan,
            "C:/Downloads/ShipLootx2-172-3-1777184245.zip",
            &metadata,
        )
        .unwrap()
        .unwrap();
        assert!(reason.contains("already installed"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unreal_pak_target_dirs_include_every_detected_game_pak_root() {
        let root = temp_game_dir("pak-roots");
        fs::create_dir_all(root.join("Content/Paks")).unwrap();
        fs::create_dir_all(root.join("R5/Content/Paks")).unwrap();
        fs::create_dir_all(root.join("R5/Builds/WindowsServer/R5/Content/Paks")).unwrap();
        fs::create_dir_all(root.join("Engine/Content/Paks")).unwrap();

        let profile = GameProfile {
            id: "profile-1".to_string(),
            name: "Test Game".to_string(),
            game_path: root.to_string_lossy().to_string(),
            game_id: None,
            steam_app_id: None,
            engine: "unreal".to_string(),
            loader: "ue4ss".to_string(),
            created_at: now_string(),
            updated_at: now_string(),
        };

        let targets = unreal_pak_target_dirs(&profile);

        assert!(targets.contains(&"Content/Paks/~mods".to_string()));
        assert!(targets.contains(&"R5/Content/Paks/~mods".to_string()));
        assert!(targets.contains(&"R5/Builds/WindowsServer/R5/Content/Paks/~mods".to_string()));
        assert!(!targets.contains(&"Engine/Content/Paks/~mods".to_string()));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn route_text_extracts_windrose_client_and_hosted_server_but_not_external_dedicated_server() {
        let root = temp_game_dir("windrose-route-text");
        fs::create_dir_all(root.join("R5")).unwrap();
        let mut profile = test_profile("windrose", "Windrose", "unreal", "ue4ss");
        profile.game_path = root.to_string_lossy().to_string();
        let text = r#"
            Solo PAK: <Windrose>\R5\Content\Paks\~mods
            Local Multiplayer: <Windrose>\R5\Builds\WindowsServer\R5\Content\Paks\~mods
            Dedicated Server: <Windrose Dedicated Server>\R5\Content\Paks\~mods
        "#;

        let candidates = extract_install_route_candidates(&profile, text);

        assert_eq!(candidates.len(), 2);
        assert!(candidates.iter().any(|candidate| {
            candidate.relative_path == "R5/Content/Paks/~mods"
                && candidate.scopes.contains(&"client".to_string())
        }));
        assert!(candidates.iter().any(|candidate| {
            candidate.relative_path == "R5/Builds/WindowsServer/R5/Content/Paks/~mods"
                && candidate.scopes.contains(&"hosted-server".to_string())
        }));
        assert!(!candidates
            .iter()
            .any(|candidate| candidate.excerpt.contains("Dedicated Server")));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn route_text_accepts_nested_ue4ss_mod_directories_from_encoded_provider_text() {
        let parent = temp_game_dir("nested-ue4ss-route-text");
        let root = parent.join("RSDragonwilds");
        fs::create_dir_all(root.join("RSDragonwilds/Binaries/Win64")).unwrap();
        let mut profile = test_profile(
            "future-unreal-game",
            "RuneScape: Dragonwilds",
            "unreal",
            "ue4ss",
        );
        profile.game_path = root.to_string_lossy().to_string();
        let text = "UE4SS is required. Extract to RSDragonwilds&#92;RSDragonwilds&#92;Binaries&#92;Win64&#92;ue4ss&#92;Mods.";

        let candidates = extract_install_route_candidates(&profile, text);

        assert!(candidates.iter().any(|candidate| {
            candidate.relative_path == "RSDragonwilds/Binaries/Win64/ue4ss/Mods"
                && candidate.adapter_id == "ue4ss"
        }));

        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn route_text_normalizes_ellipsis_and_full_steam_game_prefixes() {
        let parent = temp_game_dir("palworld-ellipsis-route-text");
        let root = parent.join("Palworld");
        fs::create_dir_all(root.join("Pal/Content/Paks")).unwrap();
        let mut profile = test_profile("future-palworld", "Palworld", "unreal", "ue4ss");
        profile.game_path = root.to_string_lossy().to_string();
        let text = r#"
            Drop files into ...\Pal\Content\Paks\~mods.
            Or use ...\steamapps\common\Palworld\Pal\Content\Paks\~mods.
        "#;

        let candidates = extract_install_route_candidates(&profile, text);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].relative_path, "Pal/Content/Paks/~mods");
        assert_eq!(candidates[0].adapter_id, "unreal-pak");

        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn existing_ellipsis_route_is_repaired_before_directory_creation() {
        let parent = temp_game_dir("palworld-existing-route-repair");
        let root = parent.join("Palworld");
        fs::create_dir_all(root.join("Pal/Content/Paks")).unwrap();
        let mut profile = test_profile("future-palworld", "Palworld", "unreal", "ue4ss");
        profile.game_path = root.to_string_lossy().to_string();
        let mut knowledge = ProfileRouteKnowledge {
            version: PROFILE_ROUTE_KNOWLEDGE_VERSION,
            profile_id: profile.id.clone(),
            learned_at: now_string(),
            sampled_mods: 1,
            providers: vec!["Nexus Mods".to_string()],
            routes: vec![LearnedInstallRoute {
                relative_path: ".../Pal/Content/Paks/~mods".to_string(),
                adapter_id: "unreal-pak".to_string(),
                scopes: vec!["general".to_string()],
                confidence: 0.9,
                supporting_mods: 1,
                providers: vec!["Nexus Mods".to_string()],
                evidence: Vec::new(),
                trusted: true,
                package_verified: false,
                created: false,
            }],
            warnings: Vec::new(),
        };

        let outcome = apply_profile_route_knowledge(&profile, &mut knowledge);

        assert!(outcome.warnings.is_empty());
        assert_eq!(knowledge.routes.len(), 1);
        assert_eq!(knowledge.routes[0].relative_path, "Pal/Content/Paks/~mods");
        assert!(root.join("Pal/Content/Paks/~mods").is_dir());

        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn ue4ss_plan_prefers_detected_nested_mod_layout_over_empty_legacy_folder() {
        let root = temp_game_dir("nested-ue4ss-plan");
        fs::create_dir_all(root.join("RSDragonwilds/Binaries/Win64/ue4ss/Mods")).unwrap();
        fs::create_dir_all(root.join("RSDragonwilds/Binaries/Win64/Mods")).unwrap();
        let mut profile = test_profile(
            "future-unreal-game",
            "Future Unreal Game",
            "unreal",
            "ue4ss",
        );
        profile.game_path = root.to_string_lossy().to_string();
        let scanned = ScannedArchive {
            archive_path: "C:/Downloads/CooldownRemover.zip".to_string(),
            archive_name: "CooldownRemover.zip".to_string(),
            entries: vec![ArchiveEntry {
                path: "Mods/CooldownRemover/Scripts/main.lua".to_string(),
                logical_path: "Mods/CooldownRemover/Scripts/main.lua".to_string(),
                size: 1,
                is_directory: false,
            }],
            manifest: None,
            package_identity: None,
        };

        let plan = ue4ss_plan(&scanned, &profile).unwrap();

        assert_eq!(plan.mappings.len(), 1);
        assert_eq!(
            plan.mappings[0].target_relative_path,
            "RSDragonwilds/Binaries/Win64/ue4ss/Mods/CooldownRemover/Scripts/main.lua"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn catalogue_routes_require_independent_mod_consensus_before_creation() {
        let root = temp_game_dir("route-consensus");
        fs::create_dir_all(root.join("R5")).unwrap();
        let mut profile = test_profile("windrose", "Windrose", "unreal", "ue4ss");
        profile.game_path = root.to_string_lossy().to_string();
        let route_text = r#"
            Single Player: Windrose\R5\Content\Paks\~mods
            Multiplayer hosting: Windrose\R5\Builds\WindowsServer\R5\Content\Paks\~mods
        "#;
        let first = ProviderRouteDocument {
            provider: "Nexus Mods".to_string(),
            mod_id: "nexus:windrose/1".to_string(),
            mod_name: "First Mod".to_string(),
            text: route_text.to_string(),
        };
        let second = ProviderRouteDocument {
            provider: "Nexus Mods".to_string(),
            mod_id: "nexus:windrose/2".to_string(),
            mod_name: "Second Mod".to_string(),
            text: route_text.to_string(),
        };

        let mut one_source =
            build_profile_route_knowledge(&profile, std::slice::from_ref(&first), Vec::new());
        let one_source_outcome = apply_profile_route_knowledge(&profile, &mut one_source);
        assert!(one_source_outcome.created_routes.is_empty());
        assert!(!root.join("R5/Content/Paks/~mods").exists());

        let mut consensus = build_profile_route_knowledge(&profile, &[first, second], Vec::new());
        let consensus_outcome = apply_profile_route_knowledge(&profile, &mut consensus);
        assert!(consensus_outcome
            .created_routes
            .contains(&"R5/Content/Paks/~mods".to_string()));
        assert!(consensus_outcome
            .created_routes
            .contains(&"R5/Builds/WindowsServer/R5/Content/Paks/~mods".to_string()));
        assert!(root.join("R5/Content/Paks/~mods").is_dir());
        assert!(root
            .join("R5/Builds/WindowsServer/R5/Content/Paks/~mods")
            .is_dir());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn selected_pak_mod_instructions_prepare_and_use_all_internal_routes() {
        let game_root = temp_game_dir("selected-pak-routes-game");
        let store_root = temp_game_dir("selected-pak-routes-store");
        fs::create_dir_all(game_root.join("R5")).unwrap();
        fs::create_dir_all(&store_root).unwrap();
        let mut profile = test_profile("windrose", "Windrose", "unreal", "ue4ss");
        profile.game_path = game_root.to_string_lossy().to_string();
        let scanned = scanned_package("MoreResources.pak", None);
        let document = ProviderRouteDocument {
            provider: "Nexus Mods".to_string(),
            mod_id: "nexus:windrose/44".to_string(),
            mod_name: "More Resources".to_string(),
            text: r#"
                Solo: <Windrose>\R5\Content\Paks\~mods
                Local Multiplayer: <Windrose>\R5\Builds\WindowsServer\R5\Content\Paks\~mods
                Dedicated Server: <Windrose Dedicated Server>\R5\Content\Paks\~mods
            "#
            .to_string(),
        };

        let outcome = prepare_package_install_routes(&store_root, &profile, &scanned, &document);
        let plan = unreal_pak_plan(&scanned, &profile).unwrap();
        let targets = plan
            .mappings
            .iter()
            .map(|mapping| mapping.target_relative_path.clone())
            .collect::<Vec<_>>();

        assert_eq!(outcome.created_routes.len(), 2);
        assert!(targets.contains(&"R5/Content/Paks/~mods/MoreResources.pak".to_string()));
        assert!(targets.contains(
            &"R5/Builds/WindowsServer/R5/Content/Paks/~mods/MoreResources.pak".to_string()
        ));
        assert_eq!(targets.len(), 2);

        let _ = fs::remove_dir_all(store_root);
        let _ = fs::remove_dir_all(game_root);
    }

    #[test]
    fn nested_loader_planners_deploy_to_every_verified_in_game_root() {
        let bepinex_root = temp_game_dir("multi-bepinex-roots");
        fs::create_dir_all(bepinex_root.join("Client/BepInEx/plugins")).unwrap();
        fs::create_dir_all(bepinex_root.join("HostedServer/BepInEx/plugins")).unwrap();
        let mut bepinex_profile = test_profile("test", "Test", "unity-mono", "bepinex");
        bepinex_profile.game_path = bepinex_root.to_string_lossy().to_string();
        let bepinex_plan = bepinex_plan(
            &scanned_package("plugins/CoolMod.dll", None),
            &bepinex_profile,
        )
        .unwrap();
        let bepinex_targets = bepinex_plan
            .mappings
            .iter()
            .map(|mapping| mapping.target_relative_path.as_str())
            .collect::<Vec<_>>();
        assert!(bepinex_targets.contains(&"Client/BepInEx/plugins/CoolMod.dll"));
        assert!(bepinex_targets.contains(&"HostedServer/BepInEx/plugins/CoolMod.dll"));

        let reframework_root = temp_game_dir("multi-reframework-roots");
        fs::create_dir_all(reframework_root.join("Client/reframework/autorun")).unwrap();
        fs::create_dir_all(reframework_root.join("HostedServer/reframework/autorun")).unwrap();
        let mut reframework_profile = test_profile("test", "Test", "re-engine", "reframework");
        reframework_profile.game_path = reframework_root.to_string_lossy().to_string();
        let reframework_plan = reframework_plan(
            &scanned_package("scripts/CoolMod.lua", None),
            &reframework_profile,
        )
        .unwrap();
        let reframework_targets = reframework_plan
            .mappings
            .iter()
            .map(|mapping| mapping.target_relative_path.as_str())
            .collect::<Vec<_>>();
        assert!(reframework_targets.contains(&"Client/reframework/autorun/CoolMod.lua"));
        assert!(reframework_targets.contains(&"HostedServer/reframework/autorun/CoolMod.lua"));

        let _ = fs::remove_dir_all(reframework_root);
        let _ = fs::remove_dir_all(bepinex_root);
    }

    #[test]
    fn description_requirements_add_known_runtimes_without_accepting_negated_mentions() {
        let profile = test_profile("windrose", "Windrose", "unreal", "ue4ss");

        let required = description_runtime_dependencies(
            &profile,
            "Requirements: UE4SS must be installed before this mod.",
            None,
        );
        let negated = description_runtime_dependencies(
            &profile,
            "This mod does not require UE4SS and works without it.",
            None,
        );

        assert_eq!(required.len(), 1);
        assert_eq!(required[0].id, "runtime:ue4ss");
        assert!(negated.is_empty());
    }

    #[test]
    fn detection_prepares_valheim_bepinex_routes_without_marking_loader_installed() {
        let root = temp_game_dir("valheim-routes");
        touch(&root, "Valheim.exe");
        touch(&root, "UnityPlayer.dll");
        touch(&root, "valheim_Data/Managed/Assembly-CSharp.dll");

        let result = detect_game_setup_impl(&root).unwrap();

        assert_eq!(result.game_id.as_deref(), Some("valheim"));
        assert_eq!(result.loader, "bepinex");
        assert!(!result.loader_installed);
        assert!(result
            .created_mod_folders
            .contains(&"BepInEx/plugins".to_string()));
        assert!(result
            .created_mod_folders
            .contains(&"BepInEx/config".to_string()));
        assert!(root.join("BepInEx/plugins").is_dir());
        assert!(root.join("BepInEx/config").is_dir());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn detection_prepares_unreal_pak_and_ue4ss_routes() {
        let root = temp_game_dir("unreal-routes");
        touch(&root, "windrose.exe");
        touch(&root, "R5/Binaries/Win64/Windrose-Win64-Shipping.exe");
        touch(&root, "R5/Content/Paks/Game.pak");

        let result = detect_game_setup_impl(&root).unwrap();

        assert_eq!(result.engine, "unreal");
        assert_eq!(result.game_id.as_deref(), Some("windrose"));
        assert!(result
            .created_mod_folders
            .contains(&"R5/Content/Paks/~mods".to_string()));
        assert!(result
            .created_mod_folders
            .contains(&"R5/Binaries/Win64/Mods".to_string()));
        assert!(root.join("R5/Content/Paks/~mods").is_dir());
        assert!(root.join("R5/Binaries/Win64/Mods").is_dir());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unknown_games_detect_nested_unreal_layout_without_guessing_a_loader() {
        let root = temp_game_dir("nested-unknown-unreal");
        touch(
            &root,
            "GameWrapper/Project/Binaries/Win64/Project-Win64-Shipping.exe",
        );
        touch(&root, "GameWrapper/Project/Content/Paks/Game.pak");

        let result = detect_game_setup_impl(&root).unwrap();

        assert_eq!(result.game_id, None);
        assert_eq!(result.engine, "unreal");
        assert_eq!(result.loader, "none");
        assert_eq!(result.recommended_loader, "none");
        assert!(!result.loader_installed);
        assert!(!result
            .warnings
            .iter()
            .any(|warning| warning.contains("Engine could not be identified")));
        assert!(result
            .created_mod_folders
            .contains(&"GameWrapper/Project/Content/Paks/~mods".to_string()));
        assert!(!root
            .join("GameWrapper/Project/Binaries/Win64/Mods")
            .exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn nested_unity_mono_and_bepinex_layout_is_detected() {
        let root = temp_game_dir("nested-unity-mono");
        touch(&root, "GameWrapper/Project/UnityPlayer.dll");
        touch(
            &root,
            "GameWrapper/Project/Project_Data/Managed/Assembly-CSharp.dll",
        );
        touch(&root, "GameWrapper/Project/BepInEx/core/BepInEx.dll");

        let result = detect_game_setup_impl(&root).unwrap();

        assert_eq!(result.engine, "unity-mono");
        assert_eq!(result.loader, "bepinex");
        assert!(result.loader_installed);
        assert!(result
            .expected_mod_folders
            .contains(&"GameWrapper/Project/BepInEx/plugins".to_string()));
        assert!(!root.join("BepInEx/plugins").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn nested_unity_il2cpp_and_bepinex_layout_is_detected() {
        let root = temp_game_dir("nested-unity-il2cpp");
        touch(&root, "GameWrapper/Project/UnityPlayer.dll");
        touch(&root, "GameWrapper/Project/GameAssembly.dll");
        touch(
            &root,
            "GameWrapper/Project/Project_Data/il2cpp_data/Metadata/global-metadata.dat",
        );
        touch(&root, "GameWrapper/Project/BepInEx/core/BepInEx.dll");
        touch(
            &root,
            "GameWrapper/Project/BepInEx/interop/Assembly-CSharp.dll",
        );

        let result = detect_game_setup_impl(&root).unwrap();

        assert_eq!(result.engine, "unity-il2cpp");
        assert_eq!(result.loader, "bepinex-il2cpp");
        assert!(result.loader_installed);
        assert!(result
            .expected_mod_folders
            .contains(&"GameWrapper/Project/BepInEx/plugins".to_string()));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn nested_re_engine_and_reframework_layout_is_detected() {
        let root = temp_game_dir("nested-re-engine");
        touch(&root, "GameWrapper/Project/re_chunk_000.pak");
        touch(&root, "GameWrapper/Project/dinput8.dll");
        touch(&root, "GameWrapper/Project/reframework/config.ini");

        let result = detect_game_setup_impl(&root).unwrap();

        assert_eq!(result.engine, "re-engine");
        assert_eq!(result.loader, "reframework");
        assert!(result.loader_installed);
        assert!(result
            .expected_mod_folders
            .contains(&"GameWrapper/Project/reframework/plugins".to_string()));
        assert!(!root.join("reframework/plugins").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn nested_unreal_ue4ss_and_native_script_layouts_are_detected() {
        let ue4ss_root = temp_game_dir("nested-unreal-ue4ss");
        touch(&ue4ss_root, "GameWrapper/Project/Binaries/Win64/UE4SS.dll");
        touch(&ue4ss_root, "GameWrapper/Project/Content/Paks/Game.pak");

        let ue4ss = detect_game_setup_impl(&ue4ss_root).unwrap();
        assert_eq!(ue4ss.engine, "unreal");
        assert_eq!(ue4ss.loader, "ue4ss");
        assert!(ue4ss.loader_installed);
        assert!(ue4ss
            .expected_mod_folders
            .contains(&"GameWrapper/Project/Binaries/Win64/Mods".to_string()));

        let script_root = temp_game_dir("nested-unreal-native-scripts");
        touch(&script_root, "GameWrapper/Project/Content/Paks/Game.pak");
        touch(
            &script_root,
            "GameWrapper/Project/Hercules/Script/Mods/InventorySort.as",
        );

        let scripts = detect_game_setup_impl(&script_root).unwrap();
        assert_eq!(scripts.engine, "unreal");
        assert_eq!(scripts.loader, "loose-files");
        assert!(scripts.loader_installed);
        assert!(scripts
            .expected_mod_folders
            .contains(&"GameWrapper/Project/Hercules/Script/Mods".to_string()));

        let _ = fs::remove_dir_all(ue4ss_root);
        let _ = fs::remove_dir_all(script_root);
    }

    #[test]
    fn game_definition_manifest_drives_detection_and_runtime_dependency() {
        let root = temp_game_dir("manifest-game-detection");
        touch(&root, "RE4.exe");
        touch(&root, "re_chunk_000.pak");

        let result = detect_game_setup_impl(&root).unwrap();
        assert_eq!(result.game_id.as_deref(), Some("re4"));

        let profile = GameProfile {
            id: "profile-1".to_string(),
            name: "Resident Evil 4".to_string(),
            game_path: root.to_string_lossy().to_string(),
            game_id: result.game_id,
            steam_app_id: None,
            engine: result.engine,
            loader: result.loader,
            created_at: now_string(),
            updated_at: now_string(),
        };
        let dependencies = profile_bootstrap_dependencies(&profile);

        assert_eq!(dependencies.len(), 1);
        assert_eq!(dependencies[0].provider, "github-release");
        assert_eq!(
            dependencies[0].source.as_deref(),
            Some("github-release:praydog/REFramework#RE4.zip")
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn installed_mod_source_is_managed_and_can_reenable_after_original_is_removed() {
        let store_root = temp_game_dir("managed-store");
        let game_root = temp_game_dir("managed-game");
        let import_root = temp_game_dir("managed-import");
        touch(&import_root, "BepInEx/plugins/CoolMod.dll");

        let profile = GameProfile {
            id: "profile-1".to_string(),
            name: "Valheim".to_string(),
            game_path: game_root.to_string_lossy().to_string(),
            game_id: Some("valheim".to_string()),
            steam_app_id: None,
            engine: "unity-mono".to_string(),
            loader: "bepinex".to_string(),
            created_at: now_string(),
            updated_at: now_string(),
        };
        let plan = InstallPlan {
            adapter_id: "bepinex".to_string(),
            adapter_name: "BepInEx".to_string(),
            confidence: 0.95,
            summary: "Install one plugin.".to_string(),
            mappings: vec![mapping(
                "BepInEx/plugins/CoolMod.dll",
                "game",
                "BepInEx/plugins/CoolMod.dll",
                "Test plugin.",
            )],
            dependencies: Vec::new(),
            warnings: Vec::new(),
            requires_confirmation: false,
        };

        let result = install_archive_impl(
            &store_root,
            &profile,
            &import_root.to_string_lossy(),
            Some("CoolModFolder"),
            None,
            &plan,
        )
        .unwrap();
        let store = read_store::<InstalledModRecord>(&installed_mods_path(&store_root)).unwrap();
        let record = store
            .items
            .iter()
            .find(|record| record.id == result.installed_mod_id)
            .cloned()
            .unwrap();

        assert!(record
            .archive_path
            .replace('\\', "/")
            .contains("/packages/"));
        assert!(Path::new(&record.archive_path).exists());
        assert!(game_root.join("BepInEx/plugins/CoolMod.dll").is_file());

        fs::remove_dir_all(&import_root).unwrap();
        deactivate_mod_files(&store_root, &profile, &record).unwrap();
        assert!(!game_root.join("BepInEx/plugins/CoolMod.dll").exists());

        let mut record_for_enable = record.clone();
        let enable_plan = record_for_enable.plan.clone().unwrap();
        let archive_path = record_for_enable.archive_path.clone();
        deploy_mod_files(
            &store_root,
            &profile,
            &record_for_enable.id.clone(),
            &archive_path,
            &enable_plan,
            &mut record_for_enable,
        )
        .unwrap();
        assert!(game_root.join("BepInEx/plugins/CoolMod.dll").is_file());

        let _ = fs::remove_dir_all(store_root);
        let _ = fs::remove_dir_all(game_root);
    }

    #[test]
    fn thunderstore_manifest_accepts_snake_and_camel_version_fields() {
        let snake_manifest = r#"{
            "name": "ExampleMod",
            "version_number": "1.2.3",
            "website_url": "https://example.invalid",
            "dependencies": []
        }"#;
        let camel_manifest = r#"{
            "name": "ExampleMod",
            "versionNumber": "1.2.3",
            "websiteUrl": "https://example.invalid",
            "dependencies": []
        }"#;

        let snake = parse_json_allow_bom::<ThunderstoreManifest>(snake_manifest).unwrap();
        let camel = parse_json_allow_bom::<ThunderstoreManifest>(camel_manifest).unwrap();

        assert_eq!(snake.version_number, "1.2.3");
        assert_eq!(camel.version_number, "1.2.3");
        assert_eq!(snake.website_url, camel.website_url);
    }

    #[test]
    fn humanized_names_preserve_known_loader_names_and_trim_thunderstore_noise() {
        assert_eq!(
            humanize_mod_display_name("denikson-BepInExPack_Valheim-5.4.2333.zip"),
            "BepInEx Pack Valheim"
        );
        assert_eq!(
            humanize_mod_display_name("InventoryTrashButton.as"),
            "Inventory Trash Button"
        );
        assert_eq!(
            humanize_mod_display_name("LingerChanceIndicator.AS"),
            "Linger Chance Indicator"
        );
        assert_eq!(
            humanize_mod_display_name("World Map Compass as"),
            "World Map Compass"
        );
    }

    #[test]
    fn nexus_download_names_recover_the_original_mod_id() {
        assert_eq!(
            nexus_mod_id_from_download_name("ShipLootx2-172-3-1777184245.zip"),
            Some(172)
        );
        assert_eq!(
            nexus_mod_id_from_download_name("1 Mil Fast Travel Bells-287-1-1777707453.zip"),
            Some(287)
        );
        assert_eq!(
            nexus_mod_id_from_download_name("ordinary-manual-mod.zip"),
            None
        );
    }

    #[test]
    fn legacy_artwork_matching_ignores_trailing_mod_variants() {
        assert_eq!(
            legacy_mod_artwork_search_term("More Stacks 2x").as_deref(),
            Some("More Stacks")
        );
        assert_eq!(
            legacy_mod_artwork_match_key("More Stacks 2x"),
            legacy_mod_artwork_match_key("MoreStacks")
        );
        assert_ne!(
            legacy_mod_artwork_match_key("More Stacks 2x"),
            legacy_mod_artwork_match_key("More Tree Resources")
        );
    }

    #[test]
    fn generated_config_files_match_installed_plugin_files() {
        let root = temp_game_dir("config-matching");
        let config_dir = root.join("BepInEx/config");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("org.bepinex.plugins.bigger_item_Stack.cfg"),
            "",
        )
        .unwrap();
        fs::write(config_dir.join("BepInEx.cfg"), "").unwrap();

        let profile = GameProfile {
            id: "profile-1".to_string(),
            name: "Valheim".to_string(),
            game_path: root.to_string_lossy().to_string(),
            game_id: Some("valheim".to_string()),
            steam_app_id: None,
            engine: "unity-mono".to_string(),
            loader: "bepinex".to_string(),
            created_at: now_string(),
            updated_at: now_string(),
        };
        let record = InstalledModRecord {
            id: "mod-1".to_string(),
            profile_id: profile.id.clone(),
            archive_path: "BiggerItemStack.zip".to_string(),
            archive_name: "BiggerItemStack-9-0-1-3-0.zip".to_string(),
            display_name: Some("Bigger Item Stack".to_string()),
            package_id: None,
            dependency_string: None,
            icon_url: None,
            adapter_id: "bepinex".to_string(),
            summary: String::new(),
            installed_at: now_string(),
            files_written: vec![root
                .join("BepInEx/plugins/BiggerItemStack.dll")
                .to_string_lossy()
                .to_string()],
            backups_written: Vec::new(),
            written_file_hashes: HashMap::new(),
            dependencies: Vec::new(),
            config_files: Vec::new(),
            runtime_id: None,
            externally_managed: false,
            enabled: true,
            last_status: "installed".to_string(),
            plan: None,
        };

        let discovered = discover_profile_config_files(&profile);
        let matched = resolved_config_files_for_record(&profile, &record, &discovered);

        assert_eq!(matched.len(), 1);
        assert!(matched[0].contains("bigger_item_Stack.cfg"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn localization_resources_are_not_exposed_as_mod_configuration() {
        let root = temp_game_dir("localization-config-filter");
        let config_dir = root.join("BepInEx/config");
        let translation_path = config_dir.join("TherzieTranslations/Warfare/Warfare.Chinese.yml");
        let root_translation_path = config_dir.join("Warfare.English.yml");
        let settings_path = config_dir.join("Warfare.yml");
        touch(
            &root,
            "BepInEx/config/TherzieTranslations/Warfare/Warfare.Chinese.yml",
        );
        touch(&root, "BepInEx/config/Warfare.English.yml");
        touch(&root, "BepInEx/config/Warfare.yml");

        let mut profile = test_profile("valheim", "Valheim", "unity-mono", "bepinex");
        profile.game_path = root.to_string_lossy().to_string();
        let record = InstalledModRecord {
            id: "warfare-mod".to_string(),
            profile_id: profile.id.clone(),
            archive_path: "Warfare.zip".to_string(),
            archive_name: "Warfare.zip".to_string(),
            display_name: Some("Warfare".to_string()),
            package_id: None,
            dependency_string: None,
            icon_url: None,
            adapter_id: "bepinex".to_string(),
            summary: String::new(),
            installed_at: now_string(),
            files_written: Vec::new(),
            backups_written: Vec::new(),
            written_file_hashes: HashMap::new(),
            dependencies: Vec::new(),
            config_files: vec![translation_path.to_string_lossy().to_string()],
            runtime_id: None,
            externally_managed: false,
            enabled: true,
            last_status: "installed".to_string(),
            plan: None,
        };

        let discovered = discover_profile_config_files(&profile);
        let resolved = resolved_config_files_for_record(&profile, &record, &discovered);
        let discovered = discovered
            .iter()
            .map(|path| normalize_filesystem_identity(path))
            .collect::<Vec<_>>();
        let resolved = resolved
            .iter()
            .map(|path| normalize_filesystem_identity(path))
            .collect::<Vec<_>>();
        let settings_path = normalize_filesystem_identity(settings_path.to_string_lossy().as_ref());
        let translation_path =
            normalize_filesystem_identity(translation_path.to_string_lossy().as_ref());
        let root_translation_path =
            normalize_filesystem_identity(root_translation_path.to_string_lossy().as_ref());

        assert!(discovered.contains(&settings_path));
        assert!(!discovered.contains(&translation_path));
        assert!(!discovered.contains(&root_translation_path));
        assert_eq!(resolved, vec![settings_path]);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn key_value_config_parser_extracts_bepinex_metadata() {
        let content = r#"
            [General]
            ## Enables the backpack.
            # Setting type: Boolean
            # Default value: true
            Enabled = false

            ## Maximum carried slots.
            # Setting type: Int32
            # Default value: 12
            Slots = 20
        "#;

        let entries = parse_key_value_config_entries(content);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].section.as_deref(), Some("General"));
        assert_eq!(entries[0].key, "Enabled");
        assert_eq!(entries[0].value, "false");
        assert_eq!(entries[0].value_type.as_deref(), Some("Boolean"));
        assert_eq!(entries[0].default_value.as_deref(), Some("true"));
        assert_eq!(
            entries[0].description.as_deref(),
            Some("Enables the backpack.")
        );
        assert_eq!(entries[1].key, "Slots");
        assert_eq!(entries[1].value_type.as_deref(), Some("Int32"));
    }

    #[test]
    fn key_value_config_editor_updates_selected_section_key() {
        let content = "[General]\nEnabled = true\n\n[Other]\nEnabled = true\n";
        let updated =
            update_key_value_config_content(content, Some("General"), "Enabled", "false").unwrap();

        assert!(updated.contains("[General]\nEnabled = false"));
        assert!(updated.contains("[Other]\nEnabled = true"));
    }

    #[test]
    fn toml_config_editor_preserves_comments_and_value_types() {
        let content = "# Keep this note\n[graphics]\nenabled = true\nquality = 3\n";
        let entries = parse_toml_config_entries(content);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].section.as_deref(), Some("graphics"));

        let updated =
            update_toml_config_content(content, Some("graphics"), "enabled", "false").unwrap();
        assert!(updated.contains("# Keep this note"));
        assert!(updated.contains("enabled = false"));
        assert!(update_toml_config_content(content, Some("graphics"), "quality", "high").is_err());
    }

    #[test]
    fn yaml_config_editor_updates_only_the_selected_nested_value() {
        let content = "gameplay:\n  stamina: 100\n  enabled: true\nserver:\n  enabled: false\n";
        let entries = parse_yaml_config_entries(content);
        assert_eq!(entries.len(), 3);

        let updated =
            update_yaml_config_content(content, Some("gameplay"), "enabled", "false").unwrap();
        let parsed = serde_yaml::from_str::<serde_yaml::Value>(&updated).unwrap();
        let gameplay = parsed
            .get("gameplay")
            .and_then(serde_yaml::Value::as_mapping)
            .unwrap();
        let server = parsed
            .get("server")
            .and_then(serde_yaml::Value::as_mapping)
            .unwrap();
        assert_eq!(
            gameplay.get("enabled").and_then(serde_yaml::Value::as_bool),
            Some(false)
        );
        assert_eq!(
            server.get("enabled").and_then(serde_yaml::Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn detection_prepares_native_script_routes_without_ue4ss_warning() {
        let root = temp_game_dir("witchspire-native-scripts");
        touch(&root, "Witchspire.exe");
        touch(&root, "Hercules/Content/Paks/Hercules-Windows.pak");
        fs::create_dir_all(root.join("Hercules/Script")).unwrap();

        let result = detect_game_setup_impl(&root).unwrap();

        assert_eq!(result.game_id.as_deref(), Some("witchspire"));
        assert_eq!(result.engine, "unreal");
        assert_eq!(result.loader, "none");
        assert!(result
            .created_mod_folders
            .contains(&"Hercules/Script/Mods".to_string()));
        assert!(!result
            .warnings
            .iter()
            .any(|warning| warning.contains("UE4SS")));
        assert!(root.join("Hercules/Script/Mods").is_dir());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn steam_app_id_resolves_registered_game_before_file_signatures() {
        let root = temp_game_dir("steam-id-mhwilds");
        fs::create_dir_all(&root).unwrap();

        let result = detect_game_setup_with_steam_app_id(&root, Some("2246340")).unwrap();

        assert_eq!(result.game_id.as_deref(), Some("mhwilds"));
        assert_eq!(result.engine, "re-engine");
        assert_eq!(result.loader, "reframework");
        assert_eq!(result.recommended_loader, "reframework");
        assert!(!result.loader_installed);
        assert!(result
            .signals
            .iter()
            .any(|signal| signal.label == "Steam App ID match"));

        let profile = GameProfile {
            id: "profile-mhwilds".to_string(),
            name: "Monster Hunter Wilds".to_string(),
            game_path: root.to_string_lossy().to_string(),
            game_id: result.game_id,
            steam_app_id: Some("2246340".to_string()),
            engine: result.engine,
            loader: result.loader,
            created_at: now_string(),
            updated_at: now_string(),
        };
        let dependencies = profile_bootstrap_dependencies(&profile);
        assert_eq!(dependencies.len(), 1);
        assert_eq!(dependencies[0].id, "runtime:reframework");
        assert_eq!(dependencies[0].provider, "github-release");
        assert_eq!(
            dependencies[0].source.as_deref(),
            Some("github-release:praydog/REFramework#MHWILDS.zip")
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    #[ignore = "live provider smoke test"]
    fn live_mhwilds_reframework_bootstrap_installs_and_verifies_runtime() {
        let root = temp_game_dir("live-mhwilds-bootstrap");
        let store_root = root.join("store");
        let game_root = root.join("game");
        fs::create_dir_all(&store_root).unwrap();
        fs::create_dir_all(&game_root).unwrap();
        let mut profile = test_profile(
            "mhwilds",
            "Monster Hunter Wilds",
            "re-engine",
            "reframework",
        );
        profile.game_path = game_root.to_string_lossy().to_string();

        let warnings = install_profile_bootstrap_dependencies(&store_root, &profile);
        assert!(
            runtime_installed(&profile, "reframework"),
            "REFramework was not detected after bootstrap: {warnings:?}"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    #[ignore = "live provider smoke test"]
    fn live_thunderstore_install_resolves_runtime_and_rejects_duplicate() {
        let root = temp_game_dir("live-thunderstore-valheim");
        let store_root = root.join("store");
        let game_root = root.join("game");
        fs::create_dir_all(&store_root).unwrap();
        fs::create_dir_all(&game_root).unwrap();
        let mut profile = test_profile("valheim", "Valheim", "unity-mono", "bepinex");
        profile.game_path = game_root.to_string_lossy().to_string();

        let result = install_thunderstore_discovered_mod(
            &store_root,
            &profile,
            "thunderstore:denikson/BepInExPack_Valheim",
            None,
            Some("valheim"),
        )
        .expect("Thunderstore BepInEx install should succeed");
        assert!(!result.files_written.is_empty());
        assert!(runtime_installed(&profile, "bepinex"));

        let store = read_store::<InstalledModRecord>(&installed_mods_path(&store_root)).unwrap();
        assert!(store.items.iter().any(|item| {
            item.profile_id == profile.id
                && item.package_id.as_deref() == Some("thunderstore:denikson/BepInExPack_Valheim")
        }));

        let duplicate = install_thunderstore_discovered_mod(
            &store_root,
            &profile,
            "thunderstore:denikson/BepInExPack_Valheim",
            None,
            Some("valheim"),
        )
        .expect_err("installing the same Thunderstore package twice should fail");
        assert!(duplicate.contains("already installed"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_archives_supply_their_own_dependency_across_registered_loaders() {
        let cases = vec![
            (
                test_profile("valheim", "Valheim", "unity-mono", "bepinex"),
                "bepinex",
                vec!["BepInEx/core/BepInEx.dll"],
            ),
            (
                test_profile(
                    "unity-il2cpp-test",
                    "Unity IL2CPP Test",
                    "unity-il2cpp",
                    "bepinex-il2cpp",
                ),
                "bepinex-il2cpp",
                vec![
                    "BepInEx/core/BepInEx.dll",
                    "BepInEx/interop/Assembly-CSharp.dll",
                ],
            ),
            (
                test_profile("windrose", "Windrose", "unreal", "ue4ss"),
                "ue4ss",
                vec!["UE4SS.dll"],
            ),
            (
                test_profile(
                    "mhwilds",
                    "Monster Hunter Wilds",
                    "re-engine",
                    "reframework",
                ),
                "reframework",
                vec!["dinput8.dll", "reframework/autorun/runtime.lua"],
            ),
        ];

        for (profile, runtime, targets) in cases {
            let plan = InstallPlan {
                adapter_id: profile.loader.clone(),
                adapter_name: runtime.to_string(),
                confidence: 1.0,
                summary: "runtime fixture".to_string(),
                mappings: targets
                    .iter()
                    .map(|target| mapping(target, "game", target, "runtime fixture"))
                    .collect(),
                dependencies: vec![known_runtime_dependency(&profile, runtime)],
                warnings: Vec::new(),
                requires_confirmation: false,
            };

            assert_eq!(
                runtime_supplied_by_plan(&profile, &plan).as_deref(),
                Some(runtime)
            );
        }

        let ordinary_mods = vec![
            (
                test_profile("valheim", "Valheim", "unity-mono", "bepinex"),
                vec!["BepInEx/plugins/CoolMod.dll"],
            ),
            (
                test_profile("windrose", "Windrose", "unreal", "ue4ss"),
                vec!["UE4SS/Mods/CoolMod/Scripts/main.lua"],
            ),
            (
                test_profile(
                    "mhwilds",
                    "Monster Hunter Wilds",
                    "re-engine",
                    "reframework",
                ),
                vec!["reframework/autorun/cool_mod.lua"],
            ),
        ];

        for (profile, targets) in ordinary_mods {
            let plan = InstallPlan {
                adapter_id: profile.loader.clone(),
                adapter_name: profile.loader.clone(),
                confidence: 1.0,
                summary: "ordinary mod fixture".to_string(),
                mappings: targets
                    .iter()
                    .map(|target| mapping(target, "game", target, "ordinary mod fixture"))
                    .collect(),
                dependencies: Vec::new(),
                warnings: Vec::new(),
                requires_confirmation: false,
            };

            assert!(runtime_supplied_by_plan(&profile, &plan).is_none());
        }
    }

    #[test]
    fn re_engine_native_assets_preserve_layout_and_strip_wrappers() {
        let profile = test_profile(
            "mhwilds",
            "Monster Hunter Wilds",
            "re-engine",
            "reframework",
        );
        let scanned = ScannedArchive {
            archive_path: "C:/Downloads/NativeAssets.zip".to_string(),
            archive_name: "NativeAssets.zip".to_string(),
            entries: vec![
                ArchiveEntry {
                    path: "Mod Wrapper/natives/STM/streaming/example.pak".to_string(),
                    logical_path: "Mod Wrapper/natives/STM/streaming/example.pak".to_string(),
                    size: 1,
                    is_directory: false,
                },
                ArchiveEntry {
                    path: "README.txt".to_string(),
                    logical_path: "README.txt".to_string(),
                    size: 1,
                    is_directory: false,
                },
            ],
            manifest: None,
            package_identity: None,
        };

        let analysis = analyze_scanned_archive(scanned.clone(), &profile);
        let plan = analysis.recommended_plan.unwrap();
        assert_eq!(plan.adapter_id, "re-engine-native");
        assert_eq!(plan.mappings.len(), 1);
        assert_eq!(
            plan.mappings[0].target_relative_path,
            "natives/STM/streaming/example.pak"
        );
        assert!(plan.dependencies.is_empty());

        let valheim = test_profile("valheim", "Valheim", "unity-mono", "bepinex");
        let blocked = analyze_scanned_archive(scanned, &valheim);
        assert!(blocked.recommended_plan.is_none());
        assert_eq!(blocked.compatibility.status, "blocked");
        assert!(blocked.compatibility.reason.contains("RE Engine"));
    }

    #[test]
    fn native_script_plan_routes_as_files_to_detected_script_mod_folder() {
        let game_root = temp_game_dir("native-script-game");
        let import_root = temp_game_dir("native-script-import");
        fs::create_dir_all(game_root.join("Hercules/Script/Mods")).unwrap();
        touch(&import_root, "Mods/ItemEditor.as");
        let profile = GameProfile {
            id: "profile-1".to_string(),
            name: "Witchspire".to_string(),
            game_path: game_root.to_string_lossy().to_string(),
            game_id: Some("witchspire".to_string()),
            steam_app_id: None,
            engine: "unreal".to_string(),
            loader: "none".to_string(),
            created_at: now_string(),
            updated_at: now_string(),
        };

        let scanned = scan_folder_source(&import_root, "ItemEditor.zip".to_string()).unwrap();
        let analysis = analyze_scanned_archive(scanned, &profile);
        let plan = analysis.recommended_plan.unwrap();

        assert_eq!(plan.adapter_id, "script-files");
        assert_eq!(plan.mappings.len(), 1);
        assert_eq!(
            plan.mappings[0].target_relative_path,
            "Hercules/Script/Mods/ItemEditor.as"
        );

        let _ = fs::remove_dir_all(game_root);
        let _ = fs::remove_dir_all(import_root);
    }

    #[test]
    fn refresh_adopts_existing_native_script_mod_files() {
        let store_root = temp_game_dir("native-script-store");
        let game_root = temp_game_dir("native-script-existing");
        touch(&game_root, "Hercules/Script/Mods/FamiliarCompendium.as");
        let profile = GameProfile {
            id: "profile-1".to_string(),
            name: "Witchspire".to_string(),
            game_path: game_root.to_string_lossy().to_string(),
            game_id: Some("witchspire".to_string()),
            steam_app_id: None,
            engine: "unreal".to_string(),
            loader: "none".to_string(),
            created_at: now_string(),
            updated_at: now_string(),
        };
        let mut store = StoreFile {
            version: 1,
            items: Vec::new(),
        };

        let adopted = adopt_existing_native_script_mods(&store_root, &profile, &mut store).unwrap();

        assert_eq!(adopted, 1);
        assert_eq!(store.items.len(), 1);
        assert_eq!(store.items[0].adapter_id, "script-files");
        assert_eq!(
            store.items[0].files_written[0].replace('\\', "/"),
            game_root
                .join("Hercules/Script/Mods/FamiliarCompendium.as")
                .to_string_lossy()
                .replace('\\', "/")
        );
        assert!(Path::new(&store.items[0].archive_path)
            .join("FamiliarCompendium.as")
            .is_file());

        let _ = fs::remove_dir_all(store_root);
        let _ = fs::remove_dir_all(game_root);
    }

    #[test]
    fn witchspire_blocks_windrose_pak_packages_even_though_both_are_unreal() {
        let profile = test_profile("witchspire", "Witchspire", "unreal", "none");
        let analysis = analyze_scanned_archive(scanned_package("WindroseMod.pak", None), &profile);

        assert!(analysis.recommended_plan.is_none());
        assert_eq!(analysis.compatibility.status, "blocked");
        assert!(analysis.compatibility.reason.contains("Witchspire"));
        assert!(analysis.compatibility.reason.contains("native script mods"));
        assert_eq!(analysis.package_identity.mod_types, vec!["unreal-pak"]);
    }

    #[test]
    fn witchspire_accepts_native_script_packages() {
        let profile = test_profile("witchspire", "Witchspire", "unreal", "none");
        let analysis = analyze_scanned_archive(scanned_package("InventorySort.as", None), &profile);

        assert_eq!(analysis.compatibility.status, "compatible");
        assert_eq!(
            analysis
                .recommended_plan
                .as_ref()
                .map(|plan| plan.adapter_id.as_str()),
            Some("script-files")
        );
    }

    #[test]
    fn windrose_accepts_unreal_pak_packages() {
        let profile = test_profile("windrose", "Windrose", "unreal", "ue4ss");
        let analysis = analyze_scanned_archive(scanned_package("FastTravel.pak", None), &profile);

        assert_eq!(analysis.compatibility.status, "compatible");
        assert_eq!(
            analysis
                .recommended_plan
                .as_ref()
                .map(|plan| plan.adapter_id.as_str()),
            Some("unreal-pak")
        );
    }

    #[test]
    fn provider_game_mismatch_blocks_otherwise_compatible_packages() {
        let profile = test_profile("valheim", "Valheim", "unity-mono", "bepinex");
        let identity = provider_source_identity(
            "nexus",
            "nexus:witchspire/44".to_string(),
            Some("1.0".to_string()),
            Some("witchspire".to_string()),
            "Nexus test fixture",
        );
        let analysis = analyze_scanned_archive_with_identity(
            scanned_package("BepInEx/plugins/Test.dll", None),
            &profile,
            Some(identity),
        );

        assert!(analysis.recommended_plan.is_none());
        assert!(analysis
            .compatibility
            .reason
            .contains("provider game 'witchspire'"));
        assert!(analysis.compatibility.reason.contains("Valheim"));
    }

    #[test]
    fn curseforge_manifests_are_identified_without_being_parsed_as_thunderstore() {
        let manifest = r#"{
          "manifestType": "minecraftModpack",
          "manifestVersion": 1,
          "name": "Example Pack",
          "version": "1.2.3",
          "minecraft": { "version": "1.20.1" },
          "files": [{ "projectID": 123, "fileID": 456 }]
        }"#;
        let (thunderstore, identity) = parse_embedded_package_manifest(manifest).unwrap();
        let identity = identity.unwrap();

        assert!(thunderstore.is_none());
        assert_eq!(identity.provider, "curseforge");
        assert_eq!(identity.provider_game_id.as_deref(), Some("minecraft"));
        assert_eq!(identity.dependencies, vec!["curseforge:123#456"]);
    }

    #[test]
    fn unknown_games_require_folder_proof_before_accepting_an_install_route() {
        let root = temp_game_dir("unknown-route-proof");
        let mut profile = test_profile("windrose", "Unknown Game", "unreal", "none");
        profile.game_id = None;
        profile.game_path = root.to_string_lossy().to_string();
        let analysis = analyze_scanned_archive(scanned_package("UnknownMod.pak", None), &profile);

        assert!(analysis.recommended_plan.is_none());
        assert!(analysis
            .compatibility
            .reason
            .contains("safe mod installation route"));

        fs::create_dir_all(root.join("Project/Content/Paks")).unwrap();
        let verified = analyze_scanned_archive(scanned_package("UnknownMod.pak", None), &profile);
        assert_eq!(verified.compatibility.status, "compatible");
        assert_eq!(
            verified
                .recommended_plan
                .as_ref()
                .map(|plan| plan.adapter_id.as_str()),
            Some("unreal-pak")
        );

        let script_package =
            analyze_scanned_archive(scanned_package("InventorySort.as", None), &profile);
        assert!(script_package.recommended_plan.is_none());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn all_registered_loader_runtimes_satisfy_equivalent_thunderstore_dependencies() {
        let bepinex_root = temp_game_dir("bepinex-runtime-equivalence");
        touch(&bepinex_root, "BepInEx/core/BepInEx.dll");
        let mut bepinex_profile = test_profile("repo", "R.E.P.O.", "unity-mono", "bepinex");
        bepinex_profile.game_path = bepinex_root.to_string_lossy().to_string();

        for name in ["BepInExPack", "BepInExPack_REPO", "BepInEx"] {
            let dependency = ThunderstorePackageRef {
                namespace: "BepInEx".to_string(),
                name: name.to_string(),
                version: Some("5.4.2304".to_string()),
            };
            assert!(thunderstore_runtime_available(
                &bepinex_profile,
                &dependency
            ));
        }

        let ordinary_plugin = ThunderstorePackageRef {
            namespace: "Azumatt".to_string(),
            name: "BepInExConfigurationManager".to_string(),
            version: Some("1.0.0".to_string()),
        };
        assert!(!thunderstore_runtime_available(
            &bepinex_profile,
            &ordinary_plugin
        ));

        let il2cpp_root = temp_game_dir("bepinex-il2cpp-runtime-equivalence");
        touch(&il2cpp_root, "BepInEx/core/BepInEx.dll");
        let mut il2cpp_profile = test_profile(
            "future-il2cpp-game",
            "Future IL2CPP Game",
            "unity-il2cpp",
            "bepinex-il2cpp",
        );
        il2cpp_profile.game_path = il2cpp_root.to_string_lossy().to_string();
        assert!(thunderstore_runtime_available(
            &il2cpp_profile,
            &ThunderstorePackageRef {
                namespace: "BepInEx".to_string(),
                name: "BepInExPack".to_string(),
                version: None,
            }
        ));

        let ue4ss_root = temp_game_dir("ue4ss-runtime-equivalence");
        touch(&ue4ss_root, "Game/Binaries/Win64/UE4SS.dll");
        let mut ue4ss_profile = test_profile("windrose", "Windrose", "unreal", "ue4ss");
        ue4ss_profile.game_path = ue4ss_root.to_string_lossy().to_string();
        assert!(thunderstore_runtime_available(
            &ue4ss_profile,
            &ThunderstorePackageRef {
                namespace: "UE4SS".to_string(),
                name: "RE-UE4SS".to_string(),
                version: None,
            }
        ));

        let reframework_root = temp_game_dir("reframework-runtime-equivalence");
        touch(&reframework_root, "dinput8.dll");
        touch(&reframework_root, "reframework/autorun/init.lua");
        let mut reframework_profile =
            test_profile("re4", "Resident Evil 4", "re-engine", "reframework");
        reframework_profile.game_path = reframework_root.to_string_lossy().to_string();
        assert!(thunderstore_runtime_available(
            &reframework_profile,
            &ThunderstorePackageRef {
                namespace: "praydog".to_string(),
                name: "REFramework".to_string(),
                version: None,
            }
        ));

        let future_runtime = DependencySpec {
            id: "runtime:future-loader".to_string(),
            name: "Future Loader".to_string(),
            version: None,
            provider: "manual".to_string(),
            required: true,
            status: "missing".to_string(),
            source: None,
            notes: None,
        };
        assert_eq!(
            runtime_from_dependency(&future_runtime),
            Some("future-loader")
        );

        let _ = fs::remove_dir_all(bepinex_root);
        let _ = fs::remove_dir_all(il2cpp_root);
        let _ = fs::remove_dir_all(ue4ss_root);
        let _ = fs::remove_dir_all(reframework_root);
    }

    #[test]
    fn detected_runtime_is_adopted_once_as_a_protected_library_record() {
        let game_root = temp_game_dir("visible-runtime-game");
        let store_root = temp_game_dir("visible-runtime-store");
        touch(&game_root, "dinput8.dll");
        touch(&game_root, "reframework/autorun/init.lua");
        fs::create_dir_all(&store_root).unwrap();

        let mut profile = test_profile("re4", "Resident Evil 4", "re-engine", "reframework");
        profile.game_path = game_root.to_string_lossy().to_string();
        let mut store = StoreFile::<InstalledModRecord> {
            version: 1,
            items: Vec::new(),
        };

        assert_eq!(
            ensure_visible_runtime_records(&store_root, &profile, &mut store).unwrap(),
            1
        );
        assert_eq!(
            ensure_visible_runtime_records(&store_root, &profile, &mut store).unwrap(),
            0
        );
        assert_eq!(store.items.len(), 1);

        let runtime = &store.items[0];
        assert_eq!(runtime.runtime_id.as_deref(), Some("reframework"));
        assert_eq!(runtime.display_name.as_deref(), Some("REFramework"));
        assert!(runtime.externally_managed);
        assert!(runtime.enabled);
        assert!(runtime
            .files_written
            .iter()
            .any(|path| path.to_lowercase().ends_with("dinput8.dll")));

        let _ = fs::remove_dir_all(game_root);
        let _ = fs::remove_dir_all(store_root);
    }

    #[test]
    fn thunderstore_runtime_inference_follows_transitive_dependencies_without_installing_them() {
        let profile = test_profile(
            "future-unity-game",
            "Future Unity Game",
            "unity-mono",
            "none",
        );
        let packages = vec![
            thunderstore_package_fixture(
                "CreatorA",
                "GameplayModA",
                50_000,
                &["Shared-CoreLibrary-1.0.0"],
            ),
            thunderstore_package_fixture(
                "CreatorB",
                "GameplayModB",
                40_000,
                &["Shared-CoreLibrary-1.0.0"],
            ),
            thunderstore_package_fixture(
                "Shared",
                "CoreLibrary",
                30_000,
                &["denikson-BepInExPack-5.4.2333"],
            ),
            thunderstore_package_fixture("denikson", "BepInExPack", 20_000, &[]),
        ];
        let mut supporters = HashMap::new();
        let mut providers = HashMap::new();

        let sampled = collect_thunderstore_runtime_votes(
            &profile,
            "future-unity-game",
            &packages,
            &mut supporters,
            &mut providers,
        );
        let inference = choose_runtime_inference(supporters, providers, sampled).unwrap();

        assert_eq!(inference.runtime_id, "bepinex");
        assert_eq!(inference.supporting_mods, 2);
        assert_eq!(inference.providers, vec!["Thunderstore"]);
    }

    #[test]
    fn runtime_inference_rejects_tied_provider_evidence() {
        let supporters = HashMap::from([
            (
                "ue4ss".to_string(),
                HashSet::from(["mod-a".to_string(), "mod-b".to_string()]),
            ),
            (
                "reframework".to_string(),
                HashSet::from(["mod-c".to_string(), "mod-d".to_string()]),
            ),
        ]);

        assert!(choose_runtime_inference(supporters, HashMap::new(), 4).is_none());
    }

    #[test]
    fn nexus_requirements_distinguish_foundation_runtime_and_optional_addons() {
        let profile = test_profile(
            "mhwilds",
            "Monster Hunter Wilds",
            "re-engine",
            "reframework",
        );
        let dependencies = nexus_requirement_dependencies(
            &profile,
            "monsterhunterwilds",
            &[
                NexusRequirement {
                    external_requirement: false,
                    game_id: "monsterhunterwilds".to_string(),
                    mod_id: "1".to_string(),
                    mod_name: "REFramework".to_string(),
                    notes: None,
                    url: "https://www.nexusmods.com/monsterhunterwilds/mods/1".to_string(),
                },
                NexusRequirement {
                    external_requirement: false,
                    game_id: "7597".to_string(),
                    mod_id: "2".to_string(),
                    mod_name: "Optional Texture Addon".to_string(),
                    notes: Some("Optional visual preset".to_string()),
                    url: String::new(),
                },
            ],
        );

        assert_eq!(dependencies.len(), 2);
        assert_eq!(dependencies[0].id, "runtime:reframework");
        assert!(dependencies[0].required);
        assert_eq!(dependencies[1].id, "nexus:monsterhunterwilds/2");
        assert!(!dependencies[1].required);
    }

    #[test]
    fn game_qualified_requirement_names_match_every_registered_runtime_safely() {
        let mono = test_profile("future-mono", "Future Mono", "unity-mono", "bepinex");
        let il2cpp = test_profile(
            "future-il2cpp",
            "Future IL2CPP",
            "unity-il2cpp",
            "bepinex-il2cpp",
        );
        let unreal = test_profile("dragonwilds", "Dragonwilds", "unreal", "ue4ss");
        let re_engine = test_profile(
            "future-re-engine",
            "Future RE Engine",
            "re-engine",
            "reframework",
        );

        assert_eq!(
            runtime_id_for_provider_package(&mono, "nexus", None, "BepInEx for Future Mono")
                .as_deref(),
            Some("bepinex")
        );
        assert_eq!(
            runtime_id_for_provider_package(&il2cpp, "nexus", None, "BepInEx for Future IL2CPP")
                .as_deref(),
            Some("bepinex-il2cpp")
        );
        assert_eq!(
            runtime_id_for_provider_package(&unreal, "nexus", None, "UE4SS for RSDragonwilds")
                .as_deref(),
            Some("ue4ss")
        );
        assert_eq!(
            runtime_id_for_provider_package(
                &re_engine,
                "nexus",
                None,
                "REFramework for Future RE Engine"
            )
            .as_deref(),
            Some("reframework")
        );
        assert_eq!(
            runtime_id_for_provider_package(&unreal, "nexus", None, "UE4SS Configuration Manager"),
            None
        );
    }

    #[test]
    fn installed_runtime_satisfies_a_game_qualified_nexus_requirement() {
        let game_root = temp_game_dir("qualified-installed-runtime");
        let store_root = temp_game_dir("qualified-installed-runtime-store");
        touch(&game_root, "RSDragonwilds/Binaries/Win64/UE4SS.dll");
        fs::create_dir_all(&store_root).unwrap();
        let mut profile = test_profile("dragonwilds", "Dragonwilds", "unreal", "ue4ss");
        profile.game_path = game_root.to_string_lossy().to_string();
        let dependencies = nexus_requirement_dependencies(
            &profile,
            "runescapedragonwilds",
            &[NexusRequirement {
                external_requirement: false,
                game_id: "7597".to_string(),
                mod_id: "4".to_string(),
                mod_name: "UE4SS for RSDragonwilds".to_string(),
                notes: None,
                url: "https://www.nexusmods.com/runescapedragonwilds/mods/4".to_string(),
            }],
        );

        assert_eq!(dependencies.len(), 1);
        assert_eq!(dependencies[0].id, "runtime:ue4ss");
        assert_eq!(
            refresh_dependency_status(&store_root, &profile, &dependencies[0]).status,
            "already-installed"
        );

        let _ = fs::remove_dir_all(game_root);
        let _ = fs::remove_dir_all(store_root);
    }

    #[test]
    fn installed_runtime_satisfies_an_anonymous_external_nexus_requirement_url() {
        let game_root = temp_game_dir("anonymous-external-runtime");
        let store_root = temp_game_dir("anonymous-external-runtime-store");
        touch(&game_root, "Pal/Binaries/Win64/UE4SS.dll");
        fs::create_dir_all(&store_root).unwrap();
        let mut profile = test_profile("palworld", "Palworld", "unreal", "ue4ss");
        profile.game_path = game_root.to_string_lossy().to_string();
        let dependencies = nexus_requirement_dependencies(
            &profile,
            "palworld",
            &[NexusRequirement {
                external_requirement: true,
                game_id: "0".to_string(),
                mod_id: "0".to_string(),
                mod_name: String::new(),
                notes: Some(String::new()),
                url: "https://github.com/Okaetsu/RE-UE4SS/releases/download/experimental-palworld/UE4SS-Palworld.zip".to_string(),
            }],
        );

        assert_eq!(dependencies.len(), 1);
        assert_eq!(dependencies[0].id, "runtime:ue4ss");
        assert_eq!(
            refresh_dependency_status(&store_root, &profile, &dependencies[0]).status,
            "already-installed"
        );

        let _ = fs::remove_dir_all(game_root);
        let _ = fs::remove_dir_all(store_root);
    }

    #[test]
    fn external_requirement_urls_match_every_registered_runtime_by_profile() {
        let cases = [
            (
                test_profile("future-mono", "Future Mono", "unity-mono", "bepinex"),
                "https://example.invalid/releases/BepInExPack-5.4.23.zip",
                "bepinex",
            ),
            (
                test_profile(
                    "future-il2cpp",
                    "Future IL2CPP",
                    "unity-il2cpp",
                    "bepinex-il2cpp",
                ),
                "https://example.invalid/releases/BepInExPack-6.0.0.zip",
                "bepinex-il2cpp",
            ),
            (
                test_profile("future-unreal", "Future Unreal", "unreal", "ue4ss"),
                "https://example.invalid/releases/UE4SS-FutureGame.zip",
                "ue4ss",
            ),
            (
                test_profile(
                    "future-re-engine",
                    "Future RE Engine",
                    "re-engine",
                    "reframework",
                ),
                "https://example.invalid/releases/REFramework-nightly.zip",
                "reframework",
            ),
        ];

        for (profile, url, expected_runtime) in cases {
            let requirement = NexusRequirement {
                external_requirement: true,
                game_id: "0".to_string(),
                mod_id: "0".to_string(),
                mod_name: String::new(),
                notes: None,
                url: url.to_string(),
            };
            assert_eq!(
                runtime_id_for_nexus_requirement(&profile, &requirement).as_deref(),
                Some(expected_runtime)
            );
        }
    }

    #[test]
    fn empty_external_nexus_requirement_rows_are_not_actionable_dependencies() {
        let profile = test_profile("future-unreal", "Future Unreal", "unreal", "ue4ss");
        let dependencies = nexus_requirement_dependencies(
            &profile,
            "future-unreal",
            &[NexusRequirement {
                external_requirement: true,
                game_id: "0".to_string(),
                mod_id: "0".to_string(),
                mod_name: String::new(),
                notes: None,
                url: String::new(),
            }],
        );

        assert!(dependencies.is_empty());
    }

    #[test]
    fn pending_nexus_handoff_uses_the_newest_existing_profile() {
        let root = temp_game_dir("pending-nexus-existing-profile");
        fs::create_dir_all(&root).unwrap();
        let current_profile = test_profile("dragonwilds", "Dragonwilds", "unreal", "ue4ss");
        write_store(
            &profiles_path(&root),
            &StoreFile {
                version: 1,
                items: vec![current_profile.clone()],
            },
        )
        .unwrap();
        let now = Utc::now().timestamp();
        write_store(
            &pending_nexus_downloads_path(&root),
            &StoreFile {
                version: 1,
                items: vec![
                    PendingNexusDownload {
                        profile_id: "deleted-profile".to_string(),
                        domain: "runescapedragonwilds".to_string(),
                        mod_id: 4,
                        file_id: 620,
                        version: None,
                        provider_game_id: "7597".to_string(),
                        created_at: now - 20,
                    },
                    PendingNexusDownload {
                        profile_id: current_profile.id.clone(),
                        domain: "runescapedragonwilds".to_string(),
                        mod_id: 4,
                        file_id: 620,
                        version: None,
                        provider_game_id: "7597".to_string(),
                        created_at: now - 10,
                    },
                ],
            },
        )
        .unwrap();
        let nxm = NexusNxmLink {
            domain: "runescapedragonwilds".to_string(),
            mod_id: 4,
            file_id: 620,
            key: "test-key".to_string(),
            expires: now + 300,
            user_id: 1,
        };

        let pending = find_pending_nexus_download(&root, &nxm).unwrap();
        assert_eq!(pending.profile_id, current_profile.id);
        let cleaned =
            read_store::<PendingNexusDownload>(&pending_nexus_downloads_path(&root)).unwrap();
        assert_eq!(cleaned.items.len(), 1);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn new_nexus_handoff_replaces_the_same_download_from_an_old_profile() {
        let root = temp_game_dir("pending-nexus-replacement");
        fs::create_dir_all(&root).unwrap();
        let now = Utc::now().timestamp();
        let pending = |profile_id: &str, created_at| PendingNexusDownload {
            profile_id: profile_id.to_string(),
            domain: "runescapedragonwilds".to_string(),
            mod_id: 4,
            file_id: 620,
            version: None,
            provider_game_id: "7597".to_string(),
            created_at,
        };

        store_pending_nexus_download(&root, pending("old-profile", now - 5), now).unwrap();
        store_pending_nexus_download(&root, pending("current-profile", now), now).unwrap();

        let store =
            read_store::<PendingNexusDownload>(&pending_nexus_downloads_path(&root)).unwrap();
        assert_eq!(store.items.len(), 1);
        assert_eq!(store.items[0].profile_id, "current-profile");

        remove_pending_nexus_downloads_for_profile(&root, "current-profile").unwrap();
        let store =
            read_store::<PendingNexusDownload>(&pending_nexus_downloads_path(&root)).unwrap();
        assert!(store.items.is_empty());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn profile_bundle_round_trip_restores_mods_configs_and_steam_identity() {
        let root = temp_game_dir("profile-bundle-round-trip");
        let source_game = root.join("source-game");
        let steam_library = root.join("steam-library");
        let target_game = steam_library
            .join("steamapps")
            .join("common")
            .join("Windrose");
        let import_source = root.join("downloads").join("FutureMod");
        let bundle_path = root.join("Windrose.uniloader-profile");
        fs::create_dir_all(source_game.join("R5/Content/Paks/~mods")).unwrap();
        touch(&source_game, "Binaries/Win64/UE4SS.dll");
        touch(&import_source, "FutureMod_P.pak");
        fs::create_dir_all(target_game.join("R5/Content/Paks/~mods")).unwrap();
        touch(&target_game, "Binaries/Win64/UE4SS.dll");
        fs::write(
            steam_library.join("steamapps/appmanifest_3041230.acf"),
            r#""AppState"
{
    "appid"        "3041230"
    "name"         "Windrose"
    "installdir"   "Windrose"
}"#,
        )
        .unwrap();

        let mut profile = test_profile("windrose", "Windrose", "unreal", "ue4ss");
        profile.game_path = source_game.to_string_lossy().to_string();
        profile.steam_app_id = Some("3041230".to_string());
        write_store(
            &profiles_path(&root),
            &StoreFile {
                version: 1,
                items: vec![profile.clone()],
            },
        )
        .unwrap();

        let scanned = scan_import_source(&root, &import_source).unwrap();
        let analysis = analyze_scanned_archive(scanned, &profile);
        let installed = install_archive_impl(
            &root,
            &profile,
            import_source.to_string_lossy().as_ref(),
            Some("FutureMod"),
            Some(analysis.package_identity),
            &analysis.recommended_plan.unwrap(),
        )
        .unwrap();
        let config_path = source_game.join("Config/FutureMod.ini");
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        fs::write(&config_path, "enabled=true\n").unwrap();
        let mut mod_store = read_store::<InstalledModRecord>(&installed_mods_path(&root)).unwrap();
        mod_store
            .items
            .iter_mut()
            .find(|record| record.id == installed.installed_mod_id)
            .unwrap()
            .config_files = vec![config_path.to_string_lossy().to_string()];
        write_store(&installed_mods_path(&root), &mod_store).unwrap();

        let exported = export_profile_bundle_impl(&root, &profile.id, &bundle_path).unwrap();
        assert_eq!(exported.exported_mods, 1);
        assert_eq!(exported.exported_config_files, 1);
        assert!(exported.warnings.is_empty());
        let transferred_bundle_path = root.join("Shared Profile Download.zip");
        fs::copy(&bundle_path, &transferred_bundle_path).unwrap();

        let installed_game = SteamGameRecord {
            app_id: "3041230".to_string(),
            name: "Windrose".to_string(),
            install_dir: target_game.to_string_lossy().to_string(),
            library_path: steam_library.to_string_lossy().to_string(),
        };
        let resolved = resolve_profile_bundle_steam_game(
            &transferred_bundle_path,
            std::slice::from_ref(&installed_game),
        )
        .unwrap();
        assert_eq!(resolved.app_id, installed_game.app_id);
        assert!(resolve_profile_bundle_steam_game(&bundle_path, &[]).is_err());

        let imported =
            import_profile_bundle_impl(&root, &transferred_bundle_path, &target_game).unwrap();
        assert_eq!(imported.profile.steam_app_id.as_deref(), Some("3041230"));
        assert_eq!(imported.installed_mods.len(), 1);
        assert_eq!(imported.config_files_written.len(), 1);
        assert!(target_game
            .join("R5/Content/Paks/~mods/FutureMod_P.pak")
            .is_file());
        assert_eq!(
            fs::read_to_string(target_game.join("Config/FutureMod.ini")).unwrap(),
            "enabled=true\n"
        );

        let _ = fs::remove_dir_all(root);
    }

    fn test_profile(game_id: &str, name: &str, engine: &str, loader: &str) -> GameProfile {
        GameProfile {
            id: format!("profile-{game_id}"),
            name: name.to_string(),
            game_path: format!("C:/Games/{name}"),
            game_id: Some(game_id.to_string()),
            steam_app_id: None,
            engine: engine.to_string(),
            loader: loader.to_string(),
            created_at: now_string(),
            updated_at: now_string(),
        }
    }

    fn thunderstore_package_fixture(
        owner: &str,
        name: &str,
        downloads: u64,
        dependencies: &[&str],
    ) -> ThunderstoreCommunityPackage {
        ThunderstoreCommunityPackage {
            name: name.to_string(),
            full_name: format!("{owner}-{name}"),
            owner: owner.to_string(),
            package_url: None,
            rating_score: 0,
            is_deprecated: false,
            has_nsfw_content: false,
            categories: Vec::new(),
            date_created: None,
            date_updated: None,
            versions: vec![ThunderstoreVersion {
                version_number: "1.0.0".to_string(),
                full_name: format!("{owner}-{name}-1.0.0"),
                download_url: "https://example.invalid/package.zip".to_string(),
                dependencies: dependencies.iter().map(|value| value.to_string()).collect(),
                description: String::new(),
                icon: None,
                downloads,
                website_url: None,
                is_active: true,
                file_size: None,
                date_created: None,
            }],
        }
    }

    fn scanned_package(relative_path: &str, identity: Option<PackageIdentity>) -> ScannedArchive {
        ScannedArchive {
            archive_path: format!("C:/Downloads/{}", basename(relative_path)),
            archive_name: basename(relative_path),
            entries: vec![ArchiveEntry {
                path: relative_path.to_string(),
                logical_path: relative_path.to_string(),
                size: 1,
                is_directory: false,
            }],
            manifest: None,
            package_identity: identity,
        }
    }

    fn temp_game_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("uniloader-{}-{}", name, Uuid::new_v4()))
    }

    fn touch(root: &Path, relative_path: &str) {
        let path = root.join(relative_path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "").unwrap();
    }
}
