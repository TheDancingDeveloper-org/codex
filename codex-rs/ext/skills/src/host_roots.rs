use std::collections::HashSet;
use std::io;
use std::sync::Arc;

use codex_config::ConfigLayerSource;
use codex_config::ConfigLayerStack;
use codex_config::SkillsConfig;
use codex_config::default_project_root_markers;
use codex_config::merge_toml_values;
use codex_config::project_root_markers_from_config;
use codex_exec_server::ExecutorFileSystem;
use codex_exec_server::GetMetadataOptions;
use codex_exec_server::LOCAL_FS;
use codex_protocol::protocol::SkillScope;
use codex_skills::system_cache_root_dir;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_path_uri::PathUri;
use codex_utils_plugins::PluginSkillRoot;
use dirs::home_dir;
use futures::StreamExt;
use toml::Value as TomlValue;

use crate::loader::HostSkillRoot;

const AGENTS_DIR_NAME: &str = ".agents";
const SKILLS_DIR_NAME: &str = "skills";

/// The extra roots configured under `[skills]`, or none when the config is
/// absent or invalid. An invalid table warns and changes nothing.
fn extra_skill_roots(config_layer_stack: &ConfigLayerStack, project: bool) -> Vec<String> {
    let effective = config_layer_stack.effective_config();
    let Some(value) = effective.as_table().and_then(|table| table.get("skills")) else {
        return Vec::new();
    };
    let skills: SkillsConfig = match value.clone().try_into() {
        Ok(skills) => skills,
        Err(err) => {
            tracing::warn!("invalid skills config: {err}");
            return Vec::new();
        }
    };
    if project {
        skills.extra_project_roots
    } else {
        skills.extra_user_roots
    }
}

/// Joins `relative` onto `base` and keeps it only when it stays inside `base`.
///
/// An absolute value, a `..` climb or anything else that resolves outside the
/// base is dropped, so a config cannot point skill loading at arbitrary paths.
fn confined_join(base: &AbsolutePathBuf, relative: &str) -> Option<AbsolutePathBuf> {
    let relative = relative.trim().trim_start_matches(['/', '\\']);
    if relative.is_empty() {
        return None;
    }
    let joined = base.join(relative);
    joined.starts_with(base).then_some(joined)
}
const MAX_CONCURRENT_ANCESTOR_PROBES: usize = 256;

pub(crate) async fn resolve_skill_roots(
    repository_file_system: Option<Arc<dyn ExecutorFileSystem>>,
    config_layer_stack: &ConfigLayerStack,
    cwd: &AbsolutePathBuf,
    plugin_skill_roots: Vec<PluginSkillRoot>,
    extra_skill_roots: Vec<AbsolutePathBuf>,
) -> Vec<HostSkillRoot> {
    let home_dir =
        home_dir().and_then(|path| AbsolutePathBuf::from_absolute_path_checked(path).ok());
    resolve_skill_roots_with_home_dir(
        repository_file_system,
        config_layer_stack,
        cwd,
        home_dir.as_ref(),
        plugin_skill_roots,
        extra_skill_roots,
    )
    .await
}

async fn resolve_skill_roots_with_home_dir(
    repository_file_system: Option<Arc<dyn ExecutorFileSystem>>,
    config_layer_stack: &ConfigLayerStack,
    cwd: &AbsolutePathBuf,
    home_dir: Option<&AbsolutePathBuf>,
    plugin_skill_roots: Vec<PluginSkillRoot>,
    extra_skill_roots: Vec<AbsolutePathBuf>,
) -> Vec<HostSkillRoot> {
    let mut roots =
        roots_from_layer_stack(config_layer_stack, home_dir, repository_file_system.clone());
    roots.extend(
        plugin_skill_roots
            .into_iter()
            .map(|root| HostSkillRoot::plugin(root, Arc::clone(&LOCAL_FS))),
    );
    roots.extend(
        extra_skill_roots
            .into_iter()
            .map(|path| local_root(path, SkillScope::User)),
    );
    roots.extend(repo_agents_skill_roots(repository_file_system, config_layer_stack, cwd).await);
    dedupe_skill_roots_by_path(&mut roots);
    roots
}

fn roots_from_layer_stack(
    config_layer_stack: &ConfigLayerStack,
    home_dir: Option<&AbsolutePathBuf>,
    repository_file_system: Option<Arc<dyn ExecutorFileSystem>>,
) -> Vec<HostSkillRoot> {
    let mut roots = Vec::new();

    for layer in config_layer_stack.all_layers_high_to_low() {
        let Some(config_folder) = layer.config_folder() else {
            continue;
        };

        match &layer.name {
            ConfigLayerSource::Project { .. } => {
                if let Some(repository_file_system) = &repository_file_system {
                    roots.push(HostSkillRoot::host(
                        config_folder.join(SKILLS_DIR_NAME),
                        SkillScope::Repo,
                        Arc::clone(repository_file_system),
                    ));
                }
            }
            ConfigLayerSource::User { .. } => {
                // Deprecated user skills location (`$CODEX_HOME/skills`), kept for backward
                // compatibility.
                roots.push(local_root(
                    config_folder.join(SKILLS_DIR_NAME),
                    SkillScope::User,
                ));
                for relative in extra_skill_roots(config_layer_stack, /*project*/ false) {
                    if let Some(path) = confined_join(&config_folder, &relative) {
                        roots.push(local_root(path, SkillScope::User));
                    }
                }

                if let Some(home_dir) = home_dir {
                    roots.push(local_root(
                        home_dir.join(AGENTS_DIR_NAME).join(SKILLS_DIR_NAME),
                        SkillScope::User,
                    ));
                }

                roots.push(local_root(
                    system_cache_root_dir(&config_folder),
                    SkillScope::System,
                ));
            }
            ConfigLayerSource::System { .. } => {
                roots.push(local_root(
                    config_folder.join(SKILLS_DIR_NAME),
                    SkillScope::Admin,
                ));
            }
            ConfigLayerSource::PackagedDefaults { .. }
            | ConfigLayerSource::Mdm { .. }
            | ConfigLayerSource::EnterpriseManaged { .. }
            | ConfigLayerSource::SessionFlags
            | ConfigLayerSource::LegacyManagedConfigTomlFromFile { .. }
            | ConfigLayerSource::LegacyManagedConfigTomlFromMdm => {}
        }
    }

    roots
}

fn local_root(path: AbsolutePathBuf, scope: SkillScope) -> HostSkillRoot {
    HostSkillRoot::host(path, scope, Arc::clone(&LOCAL_FS))
}

async fn repo_agents_skill_roots(
    repository_file_system: Option<Arc<dyn ExecutorFileSystem>>,
    config_layer_stack: &ConfigLayerStack,
    cwd: &AbsolutePathBuf,
) -> Vec<HostSkillRoot> {
    let Some(repository_file_system) = repository_file_system else {
        return Vec::new();
    };
    let project_root_markers = project_root_markers_from_stack(config_layer_stack);
    let project_root =
        find_project_root(repository_file_system.as_ref(), cwd, &project_root_markers).await;
    let directories = dirs_between_project_root_and_cwd(cwd, &project_root);
    let mut roots = Vec::new();
    let mut results = futures::stream::iter(directories)
        .map(|directory| {
            let repository_file_system = Arc::clone(&repository_file_system);
            async move {
                let agents_skills = directory.join(AGENTS_DIR_NAME).join(SKILLS_DIR_NAME);
                let agents_skills_uri = PathUri::from_abs_path(&agents_skills);
                let result = repository_file_system
                    .get_metadata(
                        &agents_skills_uri,
                        GetMetadataOptions::default(),
                        /*sandbox*/ None,
                    )
                    .await;
                (agents_skills, result)
            }
        })
        .buffered(MAX_CONCURRENT_ANCESTOR_PROBES);
    while let Some((agents_skills, result)) = results.next().await {
        match result {
            Ok(metadata) if metadata.is_directory => roots.push(HostSkillRoot::host(
                agents_skills,
                SkillScope::Repo,
                Arc::clone(&repository_file_system),
            )),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(
                    "failed to stat repo skills root {}: {error:#}",
                    agents_skills.display()
                );
            }
        }
    }
    for directory in dirs_between_project_root_and_cwd(cwd, &project_root) {
        for relative in extra_skill_roots(config_layer_stack, /*project*/ true) {
            let Some(path) = confined_join(&directory, &relative) else {
                continue;
            };
            let uri = PathUri::from_abs_path(&path);
            if let Ok(metadata) = repository_file_system
                .get_metadata(&uri, GetMetadataOptions::default(), /*sandbox*/ None)
                .await
                && metadata.is_directory
            {
                roots.push(HostSkillRoot::host(
                    path,
                    SkillScope::Repo,
                    Arc::clone(&repository_file_system),
                ));
            }
        }
    }
    roots
}

fn project_root_markers_from_stack(config_layer_stack: &ConfigLayerStack) -> Vec<String> {
    let mut merged = TomlValue::Table(toml::map::Map::new());
    for layer in config_layer_stack.layers_low_to_high() {
        if matches!(layer.name, ConfigLayerSource::Project { .. }) {
            continue;
        }
        merge_toml_values(&mut merged, &layer.config);
    }

    match project_root_markers_from_config(&merged) {
        Ok(Some(markers)) => markers,
        Ok(None) => default_project_root_markers(),
        Err(error) => {
            tracing::warn!("invalid project_root_markers: {error}");
            default_project_root_markers()
        }
    }
}

async fn find_project_root(
    repository_file_system: &dyn ExecutorFileSystem,
    cwd: &AbsolutePathBuf,
    project_root_markers: &[String],
) -> AbsolutePathBuf {
    if project_root_markers.is_empty() {
        return cwd.clone();
    }

    let mut probes = Vec::new();
    for ancestor in cwd.ancestors() {
        for marker in project_root_markers {
            probes.push((ancestor.clone(), ancestor.join(marker)));
        }
    }
    let mut results = futures::stream::iter(probes)
        .map(|(ancestor, marker_path)| async move {
            let marker_path_uri = PathUri::from_abs_path(&marker_path);
            let result = repository_file_system
                .get_metadata(
                    &marker_path_uri,
                    GetMetadataOptions::default(),
                    /*sandbox*/ None,
                )
                .await;
            (ancestor, marker_path, result)
        })
        .buffered(MAX_CONCURRENT_ANCESTOR_PROBES);
    while let Some((ancestor, marker_path, result)) = results.next().await {
        match result {
            Ok(_) => return ancestor,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(
                    "failed to stat project root marker {}: {error:#}",
                    marker_path.display()
                );
            }
        }
    }

    cwd.clone()
}

fn dirs_between_project_root_and_cwd(
    cwd: &AbsolutePathBuf,
    project_root: &AbsolutePathBuf,
) -> Vec<AbsolutePathBuf> {
    let mut directories = cwd
        .ancestors()
        .scan(false, |done, directory| {
            if *done {
                None
            } else {
                if &directory == project_root {
                    *done = true;
                }
                Some(directory)
            }
        })
        .collect::<Vec<_>>();
    directories.reverse();
    directories
}

fn dedupe_skill_roots_by_path(roots: &mut Vec<HostSkillRoot>) {
    let mut seen = HashSet::new();
    roots.retain(|root| seen.insert(root.path.clone()));
}

#[cfg(test)]
#[path = "host_roots_tests.rs"]
mod tests;
