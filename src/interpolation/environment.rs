use std::ffi::{OsStr, OsString};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExecutionEnvironment {
    pub(crate) variables: Vec<(OsString, OsString)>,
}

impl ExecutionEnvironment {
    pub const fn empty() -> Self {
        Self {
            variables: Vec::new(),
        }
    }

    pub fn from_variables<N, V>(variables: impl IntoIterator<Item = (N, V)>) -> Self
    where
        N: Into<OsString>,
        V: Into<OsString>,
    {
        let mut environment = Self::empty();
        for (name, value) in variables {
            environment.insert(name, value);
        }
        environment
    }

    pub fn get(&self, name: impl AsRef<OsStr>) -> Option<&OsStr> {
        let name = name.as_ref();
        self.variables
            .iter()
            .find(|(candidate, _)| environment_names_equal(candidate, name))
            .map(|(_, value)| value.as_os_str())
    }

    pub(crate) fn insert(&mut self, name: impl Into<OsString>, value: impl Into<OsString>) {
        let name = name.into();
        let value = value.into();

        if let Some((stored_name, stored_value)) = self
            .variables
            .iter_mut()
            .find(|(candidate, _)| environment_names_equal(candidate, &name))
        {
            *stored_name = name;
            *stored_value = value;
        } else {
            self.variables.push((name, value));
        }
    }
}

#[cfg(windows)]
fn environment_names_equal(left: &OsStr, right: &OsStr) -> bool {
    left.to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy())
}

#[cfg(not(windows))]
fn environment_names_equal(left: &OsStr, right: &OsStr) -> bool {
    left == right
}
