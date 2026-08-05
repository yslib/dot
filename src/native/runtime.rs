use std::collections::BTreeMap;
use std::env;

use directories::{BaseDirs, UserDirs};

use crate::interpolation::{ExecutionEnvironment, XdgPath, XdgPaths};
use crate::platform::PlatformInfo;

/// Host state used by native operations, detected independently from configuration loading.
#[derive(Clone, Debug)]
pub struct NativeRuntime {
    platform: PlatformInfo,
    environment: ExecutionEnvironment,
    xdg_paths: XdgPaths,
}

impl NativeRuntime {
    pub fn detect() -> Self {
        Self {
            platform: PlatformInfo::detect(),
            environment: capture_environment(),
            xdg_paths: XdgPaths::detect(),
        }
    }

    pub const fn platform(&self) -> &PlatformInfo {
        &self.platform
    }

    pub(crate) const fn environment(&self) -> &ExecutionEnvironment {
        &self.environment
    }

    pub(crate) const fn xdg_paths(&self) -> &XdgPaths {
        &self.xdg_paths
    }
}

pub(super) fn capture_environment() -> ExecutionEnvironment {
    ExecutionEnvironment::from_variables(env::vars_os())
}

impl XdgPaths {
    pub fn detect() -> Self {
        let mut values = BTreeMap::new();

        if let Some(base) = BaseDirs::new() {
            values.insert(XdgPath::Home, base.home_dir().to_path_buf());
            values.insert(XdgPath::Config, base.config_dir().to_path_buf());
            values.insert(XdgPath::ConfigLocal, base.config_local_dir().to_path_buf());
            values.insert(XdgPath::Data, base.data_dir().to_path_buf());
            values.insert(XdgPath::DataLocal, base.data_local_dir().to_path_buf());
            values.insert(XdgPath::Cache, base.cache_dir().to_path_buf());
            if let Some(path) = base.state_dir() {
                values.insert(XdgPath::State, path.to_path_buf());
            }
            if let Some(path) = base.runtime_dir() {
                values.insert(XdgPath::Runtime, path.to_path_buf());
            }
            if let Some(path) = base.executable_dir() {
                values.insert(XdgPath::Executable, path.to_path_buf());
            }
        }

        if let Some(user) = UserDirs::new()
            && let Some(path) = user.document_dir()
        {
            values.insert(XdgPath::Documents, path.to_path_buf());
        }

        Self { values }
    }
}
