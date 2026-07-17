use chrono::Utc;
use reqwest::blocking::Client;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};
use uuid::Uuid;
use zip::ZipArchive;

const MAX_SCAN_DEPTH: usize = 4;
const MAX_SCAN_ENTRIES: usize = 3000;
const MAX_UNREAL_PAK_ROOT_SCAN_DEPTH: usize = 9;
const MAX_UNREAL_PAK_ROOTS: usize = 64;
const MAX_DEPENDENCY_DEPTH: usize = 8;
const MAX_CONFIG_READ_BYTES: u64 = 512 * 1024;
const MAX_CONFIG_SCAN_DEPTH: usize = 5;
const MAX_PROFILE_CONFIG_FILES: usize = 500;
const MAX_DOWNLOAD_BYTES: u64 = 1024 * 1024 * 1024;
const THUNDERSTORE_API_BASE: &str = "https://thunderstore.io/api/experimental/package";
const GITHUB_API_BASE: &str = "https://api.github.com/repos";
const APP_UPDATE_REPOSITORY: &str = "Chucksterboy/UniLoader";
const BEPINBUILDS_BASE: &str = "https://builds.bepinex.dev";
const BEPINBUILDS_BEPINEX_BE: &str = "https://builds.bepinex.dev/projects/bepinex_be";
const GAME_DEFINITIONS_JSON: &str = include_str!("game_definitions.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameProfile {
    id: String,
    name: String,
    game_path: String,
    #[serde(default)]
    game_id: Option<String>,
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
    engine: String,
    loader: String,
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

#[derive(Debug, Clone)]
struct ThunderstorePackageRef {
    namespace: String,
    name: String,
    version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ThunderstorePackageResponse {
    latest: ThunderstoreVersion,
    #[serde(default)]
    versions: Vec<ThunderstoreVersion>,
}

#[derive(Debug, Clone, Deserialize)]
struct ThunderstoreVersion {
    version_number: String,
    full_name: String,
    download_url: String,
    #[serde(default)]
    dependencies: Vec<String>,
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
    executable_names: Vec<String>,
    #[serde(default)]
    path_markers: Vec<String>,
    #[serde(default)]
    bootstrap_runtimes: Vec<String>,
    #[serde(default)]
    config_roots: Vec<String>,
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
pub struct ArchiveAnalysis {
    archive_path: String,
    archive_name: String,
    entries: Vec<ArchiveEntry>,
    manifest: Option<ThunderstoreManifest>,
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
    adapter_id: String,
    summary: String,
    installed_at: String,
    files_written: Vec<String>,
    backups_written: Vec<String>,
    dependencies: Vec<DependencySpec>,
    #[serde(default)]
    config_files: Vec<String>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileRefreshResult {
    profile: GameProfile,
    detection: GameDetectionResult,
    installed_mods: Vec<InstalledModRecord>,
    mod_file_health: Vec<ModFileHealth>,
    missing_dependencies: Vec<DependencySpec>,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default)]
    minimize_to_tray_on_close: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateInfo {
    current_version: String,
    latest_version: Option<String>,
    update_available: bool,
    release_url: Option<String>,
    status: String,
    message: String,
}

#[derive(Debug, Serialize, Deserialize)]
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

#[derive(Debug)]
struct ScannedArchive {
    archive_path: String,
    archive_name: String,
    entries: Vec<ArchiveEntry>,
    manifest: Option<ThunderstoreManifest>,
}

#[tauri::command]
fn get_app_settings(app: AppHandle) -> Result<AppSettings, String> {
    read_app_settings(&store_root(&app)?)
}

#[tauri::command]
fn update_app_settings(app: AppHandle, input: AppSettings) -> Result<AppSettings, String> {
    let root = store_root(&app)?;
    write_app_settings(&root, &input)?;
    Ok(input)
}

#[tauri::command]
fn check_app_update() -> AppUpdateInfo {
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
                status: "error".to_string(),
                message: format!("GitHub release response was not readable: {error}"),
            };
        }
    };

    let latest_version = release.tag_name.trim_start_matches('v').to_string();
    let update_available = is_newer_version(&latest_version, &current_version);
    AppUpdateInfo {
        current_version: current_version.clone(),
        latest_version: Some(latest_version.clone()),
        update_available,
        release_url: release.html_url,
        status: if update_available {
            "available"
        } else {
            "up-to-date"
        }
        .to_string(),
        message: if update_available {
            format!("UniLoader v{latest_version} is available.")
        } else {
            format!("UniLoader v{current_version} is current.")
        },
    }
}

#[tauri::command]
fn list_profiles(app: AppHandle) -> Result<Vec<GameProfile>, String> {
    read_store::<GameProfile>(&profiles_path(&store_root(&app)?))
        .map(|store| store.items)
        .map_err(error_to_string)
}

#[tauri::command]
fn create_profile(app: AppHandle, input: CreateProfileInput) -> Result<GameProfile, String> {
    let trimmed_name = input.name.trim().to_string();
    if trimmed_name.is_empty() {
        return Err("Profile name is required.".to_string());
    }

    let root = store_root(&app)?;
    let path = profiles_path(&root);
    let mut store = read_store::<GameProfile>(&path).map_err(error_to_string)?;
    let now = now_string();
    let profile = GameProfile {
        id: Uuid::new_v4().to_string(),
        name: trimmed_name,
        game_path: input.game_path,
        game_id: input.game_id,
        engine: input.engine,
        loader: input.loader,
        created_at: now.clone(),
        updated_at: now,
    };

    store.items.push(profile.clone());
    write_store(&path, &store).map_err(error_to_string)?;
    fs::create_dir_all(profile_dir(&root, &profile.id)).map_err(error_to_string)?;
    Ok(profile)
}

#[tauri::command]
fn rename_profile(app: AppHandle, profile_id: String, name: String) -> Result<GameProfile, String> {
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
    let root = store_root(&app)?;
    let profiles_file = profiles_path(&root);
    let mut profiles = read_store::<GameProfile>(&profiles_file).map_err(error_to_string)?;
    let profile_index = profiles
        .items
        .iter()
        .position(|profile| profile.id == profile_id)
        .ok_or_else(|| format!("Profile not found: {}", profile_id))?;
    let profile = profiles.items.remove(profile_index);

    let installed_mods_file = installed_mods_path(&root);
    let mut installed_mods =
        read_store::<InstalledModRecord>(&installed_mods_file).map_err(error_to_string)?;
    let before_count = installed_mods.items.len();
    installed_mods
        .items
        .retain(|record| record.profile_id != profile.id);
    let removed_mod_records = before_count.saturating_sub(installed_mods.items.len());

    write_store(&profiles_file, &profiles).map_err(error_to_string)?;
    write_store(&installed_mods_file, &installed_mods).map_err(error_to_string)?;

    let mut warnings = Vec::new();
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
fn refresh_profile(app: AppHandle, profile_id: String) -> Result<ProfileRefreshResult, String> {
    let root = store_root(&app)?;
    let profiles_file = profiles_path(&root);
    let mut profiles = read_store::<GameProfile>(&profiles_file).map_err(error_to_string)?;
    let profile_index = profiles
        .items
        .iter()
        .position(|profile| profile.id == profile_id)
        .ok_or_else(|| format!("Profile not found: {}", profile_id))?;
    let mut profile = profiles.items[profile_index].clone();
    let detection = detect_game_setup_impl(Path::new(&profile.game_path))?;

    profile.game_id = detection.game_id.clone();
    profile.engine = detection.engine.clone();
    profile.loader = detection.loader.clone();
    profile.updated_at = now_string();
    profiles.items[profile_index] = profile.clone();
    write_store(&profiles_file, &profiles).map_err(error_to_string)?;

    let discovered_config_files = discover_profile_config_files(&profile);
    let installed_mods_file = installed_mods_path(&root);
    let mut installed_store =
        read_store::<InstalledModRecord>(&installed_mods_file).map_err(error_to_string)?;
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

        mod_file_health.push(mod_file_health_for_record(record));
    }

    write_store(&installed_mods_file, &installed_store).map_err(error_to_string)?;

    let installed_mods = installed_store
        .items
        .iter()
        .filter(|record| record.profile_id == profile_id)
        .cloned()
        .collect::<Vec<_>>();
    let warnings = profile_refresh_warnings(&detection, &mod_file_health, &missing_dependencies);

    Ok(ProfileRefreshResult {
        profile,
        detection,
        installed_mods,
        mod_file_health,
        missing_dependencies,
        warnings,
    })
}

#[tauri::command]
fn bootstrap_profile_dependencies(
    app: AppHandle,
    profile_id: String,
) -> Result<ProfileDependencyBootstrapResult, String> {
    let root = store_root(&app)?;
    let profile = get_profile(&root, &profile_id)?;
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
                installed_dependencies.push(dependency.name.clone());
                warnings.append(&mut dependency_warnings);
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

#[tauri::command]
fn detect_game_setup(game_path: String) -> Result<GameDetectionResult, String> {
    detect_game_setup_impl(Path::new(&game_path))
}

#[tauri::command]
fn analyze_archive_for_profile(
    app: AppHandle,
    profile_id: String,
    archive_path: String,
) -> Result<ArchiveAnalysis, String> {
    let root = store_root(&app)?;
    let profile = get_profile(&root, &profile_id)?;
    let scanned = scan_import_source(&root, Path::new(&archive_path))?;
    Ok(analyze_scanned_archive(scanned, &profile))
}

#[tauri::command]
fn install_archive(app: AppHandle, request: InstallRequest) -> Result<InstallResult, String> {
    let root = store_root(&app)?;
    let profile = get_profile(&root, &request.profile_id)?;
    install_archive_impl(
        &root,
        &profile,
        &request.archive_path,
        request.archive_name.as_deref(),
        &request.plan,
    )
}

#[tauri::command]
fn list_installed_mods(
    app: AppHandle,
    profile_id: String,
) -> Result<Vec<InstalledModRecord>, String> {
    let root = store_root(&app)?;
    let profile = get_profile(&root, &profile_id)?;
    let discovered_config_files = discover_profile_config_files(&profile);
    let items = read_store::<InstalledModRecord>(&installed_mods_path(&root))
        .map_err(error_to_string)?
        .items;
    Ok(items
        .into_iter()
        .filter(|record| record.profile_id == profile_id)
        .map(|mut record| {
            record.config_files =
                resolved_config_files_for_record(&profile, &record, &discovered_config_files);
            record.display_name = record
                .display_name
                .as_deref()
                .map(humanize_mod_display_name);
            record
        })
        .collect())
}

#[tauri::command]
fn get_mod_config_details(
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
    let root = store_root(&app)?;
    let profile = get_profile(&root, &input.profile_id)?;
    let path = PathBuf::from(&input.file_path);

    validate_config_file_for_edit(&root, &profile, &path)?;
    let content = fs::read_to_string(&path).map_err(error_to_string)?;
    let next_content = match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_lowercase())
        .as_deref()
    {
        Some("json") => update_json_config_content(
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

    fs::write(&path, next_content).map_err(error_to_string)?;
    read_mod_config_file(&root, &profile, &input.file_path)
}

#[tauri::command]
fn disable_mod(
    app: AppHandle,
    profile_id: String,
    installed_mod_id: String,
) -> Result<ModActionResult, String> {
    let root = store_root(&app)?;
    let profile = get_profile(&root, &profile_id)?;
    update_installed_mod(&root, &installed_mod_id, |record| {
        if record.profile_id != profile_id {
            return Err("Installed mod does not belong to the selected profile.".to_string());
        }

        if !record.enabled {
            return Ok(ModActionResult {
                profile_id: profile_id.clone(),
                installed_mod_id: installed_mod_id.clone(),
                status: record.last_status.clone(),
                files_changed: Vec::new(),
                warnings: vec!["Mod is already disabled.".to_string()],
            });
        }

        let files_changed = deactivate_mod_files(&root, &profile, record)?;
        record.enabled = false;
        record.last_status = "disabled".to_string();

        Ok(ModActionResult {
            profile_id: profile_id.clone(),
            installed_mod_id: installed_mod_id.clone(),
            status: "disabled".to_string(),
            files_changed,
            warnings: Vec::new(),
        })
    })
}

#[tauri::command]
fn enable_mod(
    app: AppHandle,
    profile_id: String,
    installed_mod_id: String,
) -> Result<ModActionResult, String> {
    let root = store_root(&app)?;
    let profile = get_profile(&root, &profile_id)?;
    update_installed_mod(&root, &installed_mod_id, |record| {
        if record.profile_id != profile_id {
            return Err("Installed mod does not belong to the selected profile.".to_string());
        }

        if record.enabled {
            return Ok(ModActionResult {
                profile_id: profile_id.clone(),
                installed_mod_id: installed_mod_id.clone(),
                status: record.last_status.clone(),
                files_changed: Vec::new(),
                warnings: vec!["Mod is already enabled.".to_string()],
            });
        }

        let plan = record.plan.clone().ok_or_else(|| {
            "This older install record cannot be re-enabled because it has no install plan."
                .to_string()
        })?;
        let install_id = record.id.clone();
        let archive_path = record.archive_path.clone();
        let files_changed =
            deploy_mod_files(&root, &profile, &install_id, &archive_path, &plan, record)?;
        record.enabled = true;
        record.last_status = "installed".to_string();
        record.files_written = files_changed.clone();
        record.config_files = config_files_from_paths(&record.files_written);

        Ok(ModActionResult {
            profile_id: profile_id.clone(),
            installed_mod_id: installed_mod_id.clone(),
            status: "installed".to_string(),
            files_changed,
            warnings: Vec::new(),
        })
    })
}

#[tauri::command]
fn remove_mod(
    app: AppHandle,
    profile_id: String,
    installed_mod_id: String,
) -> Result<ModActionResult, String> {
    let root = store_root(&app)?;
    let profile = get_profile(&root, &profile_id)?;
    let path = installed_mods_path(&root);
    let mut store = read_store::<InstalledModRecord>(&path).map_err(error_to_string)?;
    let record_index = store
        .items
        .iter()
        .position(|record| record.id == installed_mod_id && record.profile_id == profile_id)
        .ok_or_else(|| format!("Installed mod not found: {}", installed_mod_id))?;
    let record = store.items.remove(record_index);
    let files_changed = if record.enabled {
        deactivate_mod_files(&root, &profile, &record)?
    } else {
        Vec::new()
    };

    let receipt_path = profile_dir(&root, &profile_id)
        .join("receipts")
        .join(format!("{}.json", record.id));
    let _ = fs::remove_file(receipt_path);
    write_store(&path, &store).map_err(error_to_string)?;

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

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            setup_tray(app)?;
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
            list_profiles,
            create_profile,
            rename_profile,
            remove_profile,
            refresh_profile,
            bootstrap_profile_dependencies,
            detect_game_setup,
            analyze_archive_for_profile,
            install_archive,
            list_installed_mods,
            get_mod_config_details,
            update_mod_config_value,
            disable_mod,
            enable_mod,
            remove_mod,
            open_profile_game_folder,
            check_app_update,
            get_store_path
        ])
        .run(tauri::generate_context!())
        .expect("failed to run UniLoader");
}

fn setup_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("UniLoader")
        .show_menu_on_left_click(false)
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
                if let Some(window) = tray.app_handle().get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }

    let tray_icon = tray.build(app)?;
    app.manage(tray_icon);
    Ok(())
}

fn detect_game_setup_impl(game_path: &Path) -> Result<GameDetectionResult, String> {
    if !game_path.is_dir() {
        return Err("Selected game path must be a folder.".to_string());
    }

    let entries = walk_game_folder(game_path);
    let game_id = detect_game_id(&entries);
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
    let engine = choose_highest(&engine_scores, "unknown");
    score_loaders(&entries, &engine, &mut loader_scores, &mut signals);

    let installed_loader = choose_highest(&loader_scores, "none");
    let recommended_loader = recommend_loader(&engine);
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

        if entry.is_directory && lower_name.ends_with("_data") && entry.depth <= 2 {
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

        if !entry.is_directory && lower_name == "unityplayer.dll" && entry.depth <= 2 {
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

fn detect_game_id(entries: &[ProbeEntry]) -> Option<String> {
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

        if lower_path == "bepinex" {
            add_score(
                scores,
                signals,
                bepinex_loader,
                8,
                "BepInEx folder",
                &entry.relative_path,
            );
        }

        if !entry.is_directory && lower_path == "bepinex/core/bepinex.dll" {
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

        if lower_path == "bepinex/interop" || lower_path.starts_with("bepinex/interop/") {
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

        if lower_path.contains("/binaries/win64/mods") || lower_path.starts_with("mods/") {
            add_score(
                scores,
                signals,
                "ue4ss",
                18,
                "UE4SS mods folder",
                &entry.relative_path,
            );
        }

        if lower_path == "reframework" {
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

    if (game_id == Some("valheim") || loader == "bepinex" || loader == "bepinex-il2cpp")
        && (game_id == Some("valheim") || engine.starts_with("unity"))
    {
        push_unique_route(&mut routes, "BepInEx/plugins");
        push_unique_route(&mut routes, "BepInEx/config");
    }

    if engine == "unreal" || loader == "ue4ss" || game_id == Some("windrose") {
        for pak_root in find_unreal_pak_roots(game_path) {
            push_unique_route(&mut routes, &format!("{}/~mods", pak_root));
        }

        for win64_root in find_unreal_win64_dirs(entries) {
            push_unique_route(&mut routes, &format!("{}/Mods", win64_root));
        }
    }

    if engine == "re-engine" || loader == "reframework" || is_re_engine_game_id(game_id) {
        push_unique_route(&mut routes, "reframework/autorun");
        push_unique_route(&mut routes, "reframework/plugins");
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
    roots.dedup();
    roots
}

fn is_re_engine_game_id(game_id: Option<&str>) -> bool {
    game_id
        .and_then(game_definition_by_id)
        .and_then(|definition| definition.engine.as_deref())
        .map(|engine| engine == "re-engine")
        .unwrap_or(false)
}

fn scan_import_source(store_root: &Path, source_path: &Path) -> Result<ScannedArchive, String> {
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
    sevenz_rust2::decompress_file(archive_path, &import_dir).map_err(error_to_string)?;
    Ok(import_dir)
}

fn extract_rar_to_cache(store_root: &Path, archive_path: &Path) -> Result<PathBuf, String> {
    let import_dir = cache_import_dir(store_root, archive_path);
    fs::create_dir_all(&import_dir).map_err(error_to_string)?;
    let archive = rars::ArchiveReader::read_path(archive_path)
        .map_err(|error| format!("Could not read RAR archive: {}", error))?;
    let extraction_root = import_dir.clone();

    archive
        .extract_to(None, |meta| {
            let member_name = normalize_archive_path(&meta.name_lossy());
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

            Ok(Box::new(File::create(destination_path)?) as Box<dyn io::Write>)
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
        entries.push(ArchiveEntry {
            logical_path: to_logical_path(&file_path, common_top_folder.as_deref()),
            path: file_path,
            size: file.size(),
            is_directory: file.is_dir(),
        });
    }

    let manifest = read_manifest(archive_path, &entries)?;

    Ok(ScannedArchive {
        archive_path: archive_path.to_string_lossy().to_string(),
        archive_name: archive_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("mod.zip")
            .to_string(),
        entries,
        manifest,
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

    let manifest = read_folder_manifest(folder_path, &entries)?;

    Ok(ScannedArchive {
        archive_path: folder_path.to_string_lossy().to_string(),
        archive_name: import_name,
        entries,
        manifest,
    })
}

fn collect_folder_relative_paths(folder_path: &Path) -> Result<Vec<String>, String> {
    let mut paths = Vec::new();
    let mut queue = VecDeque::from([folder_path.to_path_buf()]);

    while let Some(current_path) = queue.pop_front() {
        let dirents = fs::read_dir(&current_path).map_err(error_to_string)?;
        for dirent in dirents {
            let dirent = dirent.map_err(error_to_string)?;
            let absolute_path = dirent.path();
            let relative_path = absolute_path
                .strip_prefix(folder_path)
                .map_err(error_to_string)
                .map(to_portable_path)?;
            if dirent.file_type().map_err(error_to_string)?.is_dir() {
                queue.push_back(absolute_path);
            } else {
                paths.push(relative_path);
            }
        }
    }

    Ok(paths)
}

fn analyze_scanned_archive(scanned: ScannedArchive, profile: &GameProfile) -> ArchiveAnalysis {
    let mut plans = vec![
        bepinex_plan(&scanned, profile),
        ue4ss_plan(&scanned, profile),
        reframework_plan(&scanned, profile),
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
    let recommended_plan = plans.first().cloned();

    ArchiveAnalysis {
        archive_path: scanned.archive_path,
        archive_name: scanned.archive_name,
        entries: scanned.entries,
        manifest: scanned.manifest,
        plans,
        recommended_plan,
    }
}

fn bepinex_plan(scanned: &ScannedArchive, profile: &GameProfile) -> Option<InstallPlan> {
    let files = installable_files(&scanned.entries);
    let mut mappings = Vec::new();
    let mut warnings = Vec::new();

    for file in &files {
        let lower_path = file.logical_path.to_lowercase();
        if let Some(relative_path) = path_after_named_segment(&file.logical_path, "bepinex") {
            mappings.push(mapping(
                &file.path,
                "game",
                &format!("BepInEx/{}", relative_path),
                "Archive contains a BepInEx folder layout.",
            ));
        } else if let Some(relative_path) =
            path_after_named_segment(&file.logical_path, "doorstop_libs")
        {
            mappings.push(mapping(
                &file.path,
                "game",
                &format!("doorstop_libs/{}", relative_path),
                "Doorstop runtime support file.",
            ));
        } else if let Some(relative_path) =
            path_after_named_segment(&file.logical_path, "unstripped_corlib")
        {
            mappings.push(mapping(
                &file.path,
                "game",
                &format!("unstripped_corlib/{}", relative_path),
                "BepInEx runtime support file.",
            ));
        } else if let Some(relative_path) = path_after_named_segment(&file.logical_path, "dotnet") {
            mappings.push(mapping(
                &file.path,
                "game",
                &format!("dotnet/{}", relative_path),
                "BepInEx bundled runtime file.",
            ));
        } else if is_bepinex_root_runtime_file(&file.logical_path) {
            mappings.push(mapping(
                &file.path,
                "game",
                &basename(&file.logical_path),
                "BepInEx bootstrap file.",
            ));
        } else if lower_path.starts_with("plugins/") {
            mappings.push(mapping(
                &file.path,
                "game",
                &format!("BepInEx/{}", file.logical_path),
                "Plugin folder maps into BepInEx/plugins.",
            ));
        } else if lower_path.starts_with("config/") || lower_path.ends_with(".cfg") {
            mappings.push(mapping(
                &file.path,
                "game",
                &format!("BepInEx/config/{}", basename(&file.logical_path)),
                "BepInEx config file.",
            ));
        } else if is_probable_bepinex_plugin_dll(&file.logical_path, profile) {
            mappings.push(mapping(
                &file.path,
                "game",
                &format!("BepInEx/plugins/{}", basename(&file.logical_path)),
                "Managed plugin DLL.",
            ));
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
            "Install {} file(s) into the BepInEx layout.",
            mappings.len()
        ),
        mappings,
        dependencies: vec![known_runtime_dependency(profile, runtime)],
        warnings,
        requires_confirmation: false,
    })
}

fn ue4ss_plan(scanned: &ScannedArchive, profile: &GameProfile) -> Option<InstallPlan> {
    let files = installable_files(&scanned.entries);
    let mut mappings = Vec::new();
    let mut warnings = Vec::new();
    let mod_folder_name = archive_stem(&scanned.archive_name);

    for file in &files {
        let lower_path = file.logical_path.to_lowercase();
        if is_ue4ss_root_runtime_file(&file.logical_path) {
            mappings.push(mapping(
                &file.path,
                "game",
                &format!("Binaries/Win64/{}", basename(&file.logical_path)),
                "UE4SS runtime bootstrap file.",
            ));
        } else if lower_path.starts_with("mods/") {
            mappings.push(mapping(
                &file.path,
                "game",
                &format!("Binaries/Win64/{}", file.logical_path),
                "UE4SS Mods folder.",
            ));
        } else if lower_path.starts_with("ue4ss/") {
            mappings.push(mapping(
                &file.path,
                "game",
                &format!("Binaries/Win64/{}", file.logical_path),
                "UE4SS runtime or configuration files.",
            ));
        } else if lower_path.contains("/scripts/") || lower_path.ends_with(".lua") {
            mappings.push(mapping(
                &file.path,
                "game",
                &format!(
                    "Binaries/Win64/Mods/{}/{}",
                    mod_folder_name, file.logical_path
                ),
                "UE4SS script file.",
            ));
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
            "Install {} file(s) into the default UE4SS layout.",
            mappings.len()
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

    for file in &files {
        let lower_path = file.logical_path.to_lowercase();
        if is_reframework_root_runtime_file(&file.logical_path) {
            mappings.push(mapping(
                &file.path,
                "game",
                &basename(&file.logical_path),
                "REFramework bootstrap/runtime file.",
            ));
        } else if lower_path.starts_with("reframework/") {
            mappings.push(mapping(
                &file.path,
                "game",
                &file.logical_path,
                "Archive already contains an REFramework folder layout.",
            ));
        } else if lower_path.ends_with(".lua") {
            mappings.push(mapping(
                &file.path,
                "game",
                &format!("reframework/autorun/{}", basename(&file.logical_path)),
                "REFramework autorun Lua script.",
            ));
        } else if lower_path.ends_with(".dll") {
            mappings.push(mapping(
                &file.path,
                "game",
                &format!("reframework/plugins/{}", basename(&file.logical_path)),
                "REFramework native plugin.",
            ));
        }
    }

    let has_signal = files.iter().any(|file| {
        let lower_path = file.logical_path.to_lowercase();
        lower_path.starts_with("reframework/")
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
            "Install {} file(s) into the REFramework layout.",
            mappings.len()
        ),
        mappings,
        dependencies: vec![known_runtime_dependency(profile, "reframework")],
        warnings,
        requires_confirmation: false,
    })
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
}

struct InstallOptions<'a> {
    metadata: InstallMetadata,
    resolve_dependencies: bool,
    visited_dependencies: &'a mut HashSet<String>,
    dependency_depth: usize,
}

fn install_archive_impl(
    store_root: &Path,
    profile: &GameProfile,
    archive_path: &str,
    archive_name: Option<&str>,
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
                ..InstallMetadata::default()
            },
            resolve_dependencies: true,
            visited_dependencies: &mut visited_dependencies,
            dependency_depth: 0,
        },
    )
}

fn install_archive_impl_with_metadata(
    store_root: &Path,
    profile: &GameProfile,
    archive_path: &str,
    plan: &InstallPlan,
    options: InstallOptions<'_>,
) -> Result<InstallResult, String> {
    let InstallOptions {
        metadata,
        resolve_dependencies,
        visited_dependencies,
        dependency_depth,
    } = options;

    if dependency_depth > MAX_DEPENDENCY_DEPTH {
        return Err("Dependency chain is too deep to install safely.".to_string());
    }

    let install_id = Uuid::new_v4().to_string();
    let installed_at = now_string();
    let original_source_path = Path::new(archive_path);
    let managed_source_path =
        materialize_import_source(store_root, profile, &install_id, original_source_path)?;
    let source_path = managed_source_path.as_path();
    let mut archive = if source_path.is_dir() {
        None
    } else {
        let archive_file = File::open(source_path).map_err(error_to_string)?;
        Some(ZipArchive::new(archive_file).map_err(error_to_string)?)
    };
    let profile_root = profile_dir(store_root, &profile.id);
    let backup_root = profile_backup_dir(store_root, &profile.id, &install_id);
    let mut files_written = Vec::new();
    let mut backups_written = Vec::new();
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

    for mapping in &plan.mappings {
        let source_file_path = if archive.is_none() {
            let source_file_path = safe_join(source_path, &mapping.source_path)?;
            if !source_file_path.is_file() {
                warnings.push(format!(
                    "Skipped missing folder entry: {}",
                    mapping.source_path
                ));
                continue;
            }
            Some(source_file_path)
        } else {
            if archive
                .as_mut()
                .and_then(|archive| archive.by_name(&mapping.source_path).ok())
                .is_none()
            {
                warnings.push(format!(
                    "Skipped missing archive entry: {}",
                    mapping.source_path
                ));
                continue;
            }
            None
        };

        let target_root = if mapping.target_root == "game" {
            PathBuf::from(&profile.game_path)
        } else {
            profile_root.clone()
        };
        let destination_path = safe_join(&target_root, &mapping.target_relative_path)?;

        if let Some(parent) = destination_path.parent() {
            fs::create_dir_all(parent).map_err(error_to_string)?;
        }

        if mapping.target_root == "game" && destination_path.exists() {
            let backup_path = safe_join(&backup_root, &mapping.target_relative_path)?;
            if let Some(parent) = backup_path.parent() {
                fs::create_dir_all(parent).map_err(error_to_string)?;
            }
            fs::copy(&destination_path, &backup_path).map_err(error_to_string)?;
            backups_written.push(backup_path.to_string_lossy().to_string());
        }

        let mut out_file = File::create(&destination_path).map_err(error_to_string)?;
        if let Some(archive) = archive.as_mut() {
            let mut zip_file = archive
                .by_name(&mapping.source_path)
                .map_err(error_to_string)?;
            io::copy(&mut zip_file, &mut out_file).map_err(error_to_string)?;
        } else if let Some(source_file_path) = source_file_path {
            let mut source_file = File::open(source_file_path).map_err(error_to_string)?;
            io::copy(&mut source_file, &mut out_file).map_err(error_to_string)?;
        }
        files_written.push(destination_path.to_string_lossy().to_string());
    }

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
    let display_name = install_display_name(store_root, &managed_archive_path, plan, &metadata);

    let record = InstalledModRecord {
        id: install_id.clone(),
        profile_id: profile.id.clone(),
        archive_path: managed_archive_path.clone(),
        archive_name,
        display_name: Some(display_name),
        package_id: metadata.package_id,
        dependency_string: metadata.dependency_string,
        adapter_id: plan.adapter_id.clone(),
        summary: plan.summary.clone(),
        installed_at: installed_at.clone(),
        files_written: files_written.clone(),
        backups_written: backups_written.clone(),
        dependencies,
        config_files: config_files_from_paths(&files_written),
        enabled: true,
        last_status: "installed".to_string(),
        plan: Some(plan.clone()),
    };

    add_installed_mod(store_root, record.clone())?;
    write_receipt(store_root, profile, &record)?;

    Ok(InstallResult {
        profile_id: profile.id.clone(),
        archive_path: managed_archive_path,
        installed_mod_id: install_id,
        installed_at,
        files_written,
        backups_written,
        warnings,
    })
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

    match runtime {
        "bepinex-il2cpp" => RuntimeDependencyDefinition {
            id: "runtime:bepinex-il2cpp".to_string(),
            name: "BepInEx Unity IL2CPP x64".to_string(),
            provider: "bepinbuilds".to_string(),
            source: "bepinbuilds:bepinex_be#BepInEx-Unity.IL2CPP-win-x64-*.zip".to_string(),
            notes: Some(
                "Official BepInEx bleeding-edge IL2CPP build for Windows x64 Unity games."
                    .to_string(),
            ),
        },
        "ue4ss" => RuntimeDependencyDefinition {
            id: "runtime:ue4ss".to_string(),
            name: "UE4SS".to_string(),
            provider: "github-release".to_string(),
            source: "github-release:UE4SS-RE/RE-UE4SS#UE4SS_v*.zip".to_string(),
            notes: Some(
                "Official latest UE4SS release for Unreal Engine script and hook mods."
                    .to_string(),
            ),
        },
        "reframework" => RuntimeDependencyDefinition {
            id: "runtime:reframework".to_string(),
            name: "REFramework".to_string(),
            provider: "github-release".to_string(),
            source: "github-release:praydog/REFramework-nightly#REFramework.zip".to_string(),
            notes: Some(
                "Official REFramework release. Game-specific assets can be supplied by game definitions."
                    .to_string(),
            ),
        },
        _ => RuntimeDependencyDefinition {
            id: "runtime:bepinex".to_string(),
            name: "BepInEx Mono x64".to_string(),
            provider: "github-release".to_string(),
            source: "github-release:BepInEx/BepInEx#BepInEx_win_x64_*.zip".to_string(),
            notes: Some("Official stable BepInEx 5 Windows x64 build for Mono Unity games.".to_string()),
        },
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
        (_, "re-engine", "reframework") if is_re_engine_game_id(profile.game_id.as_deref()) => {
            vec![known_runtime_dependency(profile, "reframework")]
        }
        _ => Vec::new(),
    }
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

    if thunderstore_package_installed(store_root, profile, &package_id, Some(&dependency_string))
        || thunderstore_runtime_available(profile, &package_ref)
    {
        return Ok(Vec::new());
    }

    let archive_path = download_thunderstore_package(store_root, &package_ref, &package_version)?;
    let scanned = scan_zip_archive(&archive_path)?;
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

fn mod_file_health_for_record(record: &InstalledModRecord) -> ModFileHealth {
    let checked_files = if record.enabled {
        record.files_written.len()
    } else {
        0
    };
    let missing_files = if record.enabled {
        record
            .files_written
            .iter()
            .filter(|path| !Path::new(path).exists())
            .cloned()
            .collect()
    } else {
        Vec::new()
    };

    ModFileHealth {
        installed_mod_id: record.id.clone(),
        mod_name: record
            .display_name
            .as_deref()
            .map(humanize_mod_display_name)
            .unwrap_or_else(|| humanize_mod_display_name(&record.archive_name)),
        checked_files,
        missing_files,
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

fn runtime_from_dependency(dependency: &DependencySpec) -> Option<&'static str> {
    match dependency.id.as_str() {
        "runtime:bepinex" => Some("bepinex"),
        "runtime:bepinex-il2cpp" => Some("bepinex-il2cpp"),
        "runtime:ue4ss" => Some("ue4ss"),
        "runtime:reframework" => Some("reframework"),
        _ => None,
    }
}

fn runtime_installed(profile: &GameProfile, runtime: &str) -> bool {
    let game_path = Path::new(&profile.game_path);
    if !game_path.is_dir() {
        return false;
    }

    let entries = walk_game_folder(game_path);
    let has_bepinex_core = entries.iter().any(|entry| {
        !entry.is_directory
            && entry
                .relative_path
                .eq_ignore_ascii_case("BepInEx/core/BepInEx.dll")
    });
    let has_bepinex_folder = entries
        .iter()
        .any(|entry| entry.is_directory && entry.relative_path.eq_ignore_ascii_case("BepInEx"));
    let has_doorstop_config = entries
        .iter()
        .any(|entry| !entry.is_directory && entry.name.eq_ignore_ascii_case("doorstop_config.ini"));
    let has_winhttp = entries
        .iter()
        .any(|entry| !entry.is_directory && entry.name.eq_ignore_ascii_case("winhttp.dll"));

    match runtime {
        "bepinex" => has_bepinex_core || (has_bepinex_folder && has_doorstop_config && has_winhttp),
        "bepinex-il2cpp" => {
            let has_interop = entries.iter().any(|entry| {
                entry.relative_path.eq_ignore_ascii_case("BepInEx/interop")
                    || entry
                        .relative_path
                        .to_lowercase()
                        .starts_with("bepinex/interop/")
            });
            let has_dotnet = entries
                .iter()
                .any(|entry| entry.relative_path.eq_ignore_ascii_case("dotnet"));
            has_bepinex_core && (has_interop || has_dotnet || profile.engine == "unity-il2cpp")
        }
        "ue4ss" => entries.iter().any(|entry| {
            !entry.is_directory
                && (entry.name.eq_ignore_ascii_case("UE4SS.dll")
                    || entry.name.eq_ignore_ascii_case("UE4SS-settings.ini"))
        }),
        "reframework" => {
            let has_reframework_folder = entries.iter().any(|entry| {
                entry.relative_path.eq_ignore_ascii_case("reframework")
                    || entry
                        .relative_path
                        .to_lowercase()
                        .starts_with("reframework/")
            });
            let has_reframework_bootstrap = entries
                .iter()
                .any(|entry| !entry.is_directory && entry.name.eq_ignore_ascii_case("dinput8.dll"));
            let has_revision_file = entries.iter().any(|entry| {
                !entry.is_directory && entry.name.eq_ignore_ascii_case("reframework_revision.txt")
            });
            has_reframework_bootstrap && (has_reframework_folder || has_revision_file)
        }
        _ => false,
    }
}

fn thunderstore_runtime_available(
    profile: &GameProfile,
    package_ref: &ThunderstorePackageRef,
) -> bool {
    if package_ref.namespace.eq_ignore_ascii_case("denikson")
        && package_ref.name.eq_ignore_ascii_case("BepInExPack_Valheim")
    {
        return runtime_installed(profile, "bepinex");
    }

    false
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
    let url = format!(
        "{}/{}/{}",
        THUNDERSTORE_API_BASE, package_ref.namespace, package_ref.name
    );
    let response = client
        .get(url)
        .send()
        .map_err(error_to_string)?
        .error_for_status()
        .map_err(error_to_string)?
        .json::<ThunderstorePackageResponse>()
        .map_err(error_to_string)?;

    if let Some(requested_version) = &package_ref.version {
        response
            .versions
            .iter()
            .chain(std::iter::once(&response.latest))
            .find(|version| version.version_number == *requested_version)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "Thunderstore package {}/{} does not have version {}.",
                    package_ref.namespace, package_ref.name, requested_version
                )
            })
    } else {
        Ok(response.latest)
    }
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
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(error_to_string)?;
    }

    let temp_path = destination.with_extension("download");
    if temp_path.exists() {
        fs::remove_file(&temp_path).map_err(error_to_string)?;
    }

    let mut response = client
        .get(url)
        .send()
        .map_err(error_to_string)?
        .error_for_status()
        .map_err(error_to_string)?;

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
    let bytes_written = io::copy(&mut response, &mut file).map_err(error_to_string)?;
    file.flush().map_err(error_to_string)?;

    if bytes_written == 0 {
        let _ = fs::remove_file(&temp_path);
        return Err("Download completed, but the downloaded file was empty.".to_string());
    }

    fs::rename(&temp_path, destination).map_err(error_to_string)?;
    Ok(())
}

fn thunderstore_client() -> Result<Client, String> {
    provider_client()
}

fn provider_client() -> Result<Client, String> {
    Client::builder()
        .user_agent("UniLoader/0.1")
        .timeout(Duration::from_secs(45))
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

fn update_installed_mod<F>(
    store_root: &Path,
    installed_mod_id: &str,
    mut updater: F,
) -> Result<ModActionResult, String>
where
    F: FnMut(&mut InstalledModRecord) -> Result<ModActionResult, String>,
{
    let path = installed_mods_path(store_root);
    let mut store = read_store::<InstalledModRecord>(&path).map_err(error_to_string)?;
    let record = store
        .items
        .iter_mut()
        .find(|record| record.id == installed_mod_id)
        .ok_or_else(|| format!("Installed mod not found: {}", installed_mod_id))?;
    let result = updater(record)?;
    write_store(&path, &store).map_err(error_to_string)?;
    Ok(result)
}

fn deactivate_mod_files(
    store_root: &Path,
    profile: &GameProfile,
    record: &InstalledModRecord,
) -> Result<Vec<String>, String> {
    let mut files_changed = Vec::new();

    for file_path in &record.files_written {
        let path = PathBuf::from(file_path);
        if path.exists() {
            fs::remove_file(&path).map_err(error_to_string)?;
            files_changed.push(path.to_string_lossy().to_string());
        }
    }

    let backup_root = profile_backup_dir(store_root, &profile.id, &record.id);
    for backup_path in &record.backups_written {
        let backup = PathBuf::from(backup_path);
        if !backup.exists() {
            continue;
        }

        let Ok(relative_path) = backup.strip_prefix(&backup_root) else {
            continue;
        };
        let target = PathBuf::from(&profile.game_path).join(relative_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(error_to_string)?;
        }
        fs::copy(&backup, &target).map_err(error_to_string)?;
        files_changed.push(target.to_string_lossy().to_string());
    }

    Ok(files_changed)
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
    let mut archive = if source_path.is_dir() {
        None
    } else {
        let archive_file = File::open(source_path).map_err(error_to_string)?;
        Some(ZipArchive::new(archive_file).map_err(error_to_string)?)
    };
    let profile_root = profile_dir(store_root, &profile.id);
    let backup_root = profile_backup_dir(store_root, &profile.id, install_id);
    let mut files_written = Vec::new();

    for mapping in &plan.mappings {
        let source_file_path = if archive.is_none() {
            let source_file_path = safe_join(source_path, &mapping.source_path)?;
            if !source_file_path.is_file() {
                return Err(format!(
                    "Skipped missing folder entry: {}",
                    mapping.source_path
                ));
            }
            Some(source_file_path)
        } else {
            if archive
                .as_mut()
                .and_then(|archive| archive.by_name(&mapping.source_path).ok())
                .is_none()
            {
                return Err(format!(
                    "Skipped missing archive entry: {}",
                    mapping.source_path
                ));
            }
            None
        };

        let target_root = if mapping.target_root == "game" {
            PathBuf::from(&profile.game_path)
        } else {
            profile_root.clone()
        };
        let destination_path = safe_join(&target_root, &mapping.target_relative_path)?;

        if let Some(parent) = destination_path.parent() {
            fs::create_dir_all(parent).map_err(error_to_string)?;
        }

        if mapping.target_root == "game" && destination_path.exists() {
            let backup_path = safe_join(&backup_root, &mapping.target_relative_path)?;
            if !backup_path.exists() {
                if let Some(parent) = backup_path.parent() {
                    fs::create_dir_all(parent).map_err(error_to_string)?;
                }
                fs::copy(&destination_path, &backup_path).map_err(error_to_string)?;
                record
                    .backups_written
                    .push(backup_path.to_string_lossy().to_string());
            }
        }

        let mut out_file = File::create(&destination_path).map_err(error_to_string)?;
        if let Some(archive) = archive.as_mut() {
            let mut zip_file = archive
                .by_name(&mapping.source_path)
                .map_err(error_to_string)?;
            io::copy(&mut zip_file, &mut out_file).map_err(error_to_string)?;
        } else if let Some(source_file_path) = source_file_path {
            let mut source_file = File::open(source_file_path).map_err(error_to_string)?;
            io::copy(&mut source_file, &mut out_file).map_err(error_to_string)?;
        }
        files_written.push(destination_path.to_string_lossy().to_string());
    }

    Ok(files_written)
}

fn discover_profile_config_files(profile: &GameProfile) -> Vec<String> {
    let mut files = Vec::new();
    let mut seen = HashSet::new();

    for root in profile_config_roots(profile) {
        if root.is_dir() {
            discover_config_files_in_dir(&root, 0, &mut files, &mut seen);
        } else if root.is_file() && is_supported_config_file(&root) {
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
            if is_supported_config_file(&path) {
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
    )
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

fn resolved_config_files_for_record(
    profile: &GameProfile,
    record: &InstalledModRecord,
    discovered_config_files: &[String],
) -> Vec<String> {
    let mut files = Vec::new();
    let mut seen = HashSet::new();

    for path in &record.config_files {
        push_unique_config_path(&mut files, &mut seen, Path::new(path));
    }

    for path in discovered_config_files {
        if config_file_matches_record(profile, record, path) {
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

    if !is_supported_config_file(path) {
        return Err("This file type is not editable as a config file yet.".to_string());
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
                || lower_path.ends_with(".json")
                || lower_path.ends_with(".toml")
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
    let without_extension = file_name
        .strip_suffix(".zip")
        .or_else(|| file_name.strip_suffix(".7z"))
        .or_else(|| file_name.strip_suffix(".rar"))
        .or_else(|| file_name.strip_suffix(".pak"))
        .or_else(|| file_name.strip_suffix(".dll"))
        .or_else(|| file_name.strip_suffix(".lua"))
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
) -> Result<Option<ThunderstoreManifest>, String> {
    let Some(manifest_entry) = entries.iter().find(|entry| {
        !entry.is_directory && entry.logical_path.eq_ignore_ascii_case("manifest.json")
    }) else {
        return Ok(None);
    };

    let archive_file = File::open(archive_path).map_err(error_to_string)?;
    let mut archive = ZipArchive::new(archive_file).map_err(error_to_string)?;
    let mut manifest_file = archive
        .by_name(&manifest_entry.path)
        .map_err(error_to_string)?;
    let mut manifest_content = String::new();
    io::Read::read_to_string(&mut manifest_file, &mut manifest_content).map_err(error_to_string)?;

    parse_json_allow_bom::<ThunderstoreManifest>(&manifest_content)
        .map(Some)
        .map_err(error_to_string)
}

fn read_folder_manifest(
    folder_path: &Path,
    entries: &[ArchiveEntry],
) -> Result<Option<ThunderstoreManifest>, String> {
    let Some(manifest_entry) = entries.iter().find(|entry| {
        !entry.is_directory && entry.logical_path.eq_ignore_ascii_case("manifest.json")
    }) else {
        return Ok(None);
    };

    let manifest_path = safe_join(folder_path, &manifest_entry.path)?;
    let manifest_content = fs::read_to_string(manifest_path).map_err(error_to_string)?;
    parse_json_allow_bom::<ThunderstoreManifest>(&manifest_content)
        .map(Some)
        .map_err(error_to_string)
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
                && (child_depth <= 1 || should_descend_into(&portable_path, &name))
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
                | "~mods"
                | "mods"
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

fn recommend_loader(engine: &str) -> String {
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
    fs::write(receipt_path, format!("{}\n", content)).map_err(error_to_string)
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
    let raw = fs::read_to_string(path).map_err(error_to_string)?;
    parse_json_allow_bom::<AppSettings>(&raw).map_err(error_to_string)
}

fn write_app_settings(root: &Path, settings: &AppSettings) -> Result<(), String> {
    write_app_settings_raw(&settings_path(root), settings).map_err(error_to_string)
}

fn write_app_settings_raw(path: &Path, settings: &AppSettings) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(settings)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    fs::write(path, format!("{}\n", content))
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

fn profile_dir(root: &Path, profile_id: &str) -> PathBuf {
    root.join("profiles").join(profile_id)
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

fn ensure_store<T>(path: &Path) -> io::Result<()>
where
    T: Serialize + DeserializeOwned,
{
    if !path.exists() {
        write_store::<T>(
            path,
            &StoreFile {
                version: 1,
                items: Vec::new(),
            },
        )?;
    }
    Ok(())
}

fn read_store<T>(path: &Path) -> io::Result<StoreFile<T>>
where
    T: Serialize + DeserializeOwned,
{
    ensure_store::<T>(path)?;
    let raw = fs::read_to_string(path)?;
    parse_json_allow_bom(&raw).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn write_store<T>(path: &Path, store: &StoreFile<T>) -> io::Result<()>
where
    T: Serialize + DeserializeOwned,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(store)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    fs::write(path, format!("{}\n", content))
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

fn open_folder_in_shell(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        Command::new("explorer")
            .arg(path)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_version_comparison_handles_multi_digit_segments() {
        assert!(is_newer_version("0.10.0", "0.2.0"));
        assert!(is_newer_version("v1.0.0", "0.9.9"));
        assert!(!is_newer_version("0.1.0", "0.1.0"));
        assert!(!is_newer_version("0.1.0", "0.2.0"));
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
        touch(&root, "Game/Binaries/Win64/Game-Win64-Shipping.exe");
        touch(&root, "Game/Content/Paks/Game.pak");

        let result = detect_game_setup_impl(&root).unwrap();

        assert_eq!(result.engine, "unreal");
        assert!(result
            .created_mod_folders
            .contains(&"Game/Content/Paks/~mods".to_string()));
        assert!(result
            .created_mod_folders
            .contains(&"Game/Binaries/Win64/Mods".to_string()));
        assert!(root.join("Game/Content/Paks/~mods").is_dir());
        assert!(root.join("Game/Binaries/Win64/Mods").is_dir());

        let _ = fs::remove_dir_all(root);
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
            name: "Managed Game".to_string(),
            game_path: game_root.to_string_lossy().to_string(),
            game_id: None,
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
            adapter_id: "bepinex".to_string(),
            summary: String::new(),
            installed_at: now_string(),
            files_written: vec![root
                .join("BepInEx/plugins/BiggerItemStack.dll")
                .to_string_lossy()
                .to_string()],
            backups_written: Vec::new(),
            dependencies: Vec::new(),
            config_files: Vec::new(),
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

    fn temp_game_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("uniloader-{}-{}", name, Uuid::new_v4()))
    }

    fn touch(root: &Path, relative_path: &str) {
        let path = root.join(relative_path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "").unwrap();
    }
}
