//! uv invocations, as data.
//!
//! Building the command and running it are separate so the arguments can be
//! asserted on every platform without uv present.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Pinned rather than "latest" so a new uv default can't silently move the
/// interpreter under existing installs. MarkItDown needs >= 3.10.
pub const PYTHON_VERSION: &str = "3.12";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

fn spec(program: &Path, args: &[&str], runtime_dir: Option<&Path>) -> CommandSpec {
    let mut env = BTreeMap::new();
    env.insert("UV_NO_PROGRESS".to_string(), "1".to_string());
    if let Some(dir) = runtime_dir {
        env.insert(
            "UV_PYTHON_INSTALL_DIR".to_string(),
            dir.to_string_lossy().into_owned(),
        );
    }
    CommandSpec {
        program: program.to_path_buf(),
        args: args.iter().map(|s| s.to_string()).collect(),
        env,
    }
}

/// Download a managed interpreter into `runtime_dir`.
pub fn install_python(uv: &Path, runtime_dir: &Path) -> CommandSpec {
    spec(uv, &["python", "install", PYTHON_VERSION], Some(runtime_dir))
}

/// Create the managed venv. `interpreter` overrides the managed interpreter
/// when the user pointed the app at their own Python.
///
/// Always passes `--clear`: uv refuses outright if `venv` already exists as a
/// directory ("A virtual environment already exists at: ..."), even when it's
/// an empty shell left by an interrupted install (the app closed mid-`uv
/// venv`, antivirus quarantining `Scripts\python.exe` on Windows, a partial
/// `remove_dir_all`). Without `--clear`, that leftover directory makes every
/// retry fail with the same unhelpful message — nothing in the app can dig
/// itself out. `--clear` makes "the target must be empty" the command's own
/// guarantee, so both call sites in `ensure()` get it for free.
///
/// Always passes `--force` too, and it has to travel with `--clear` rather
/// than replace it: `--clear` is what makes uv accept a directory that's
/// already a venv; `--force` is what makes it accept one that *isn't* a venv
/// at all — a stray, non-venv directory at the target path (verified against
/// uv 0.11.19: without `--force`, `--clear` alone still succeeds there, but
/// prints a deprecation warning that uv says becomes a hard error in a future
/// release). Both leftover shapes come from the same interruption — an
/// install that dies between `remove_dir_all` clearing `pyvenv.cfg` and uv
/// finishing the rebuild, or an antivirus quarantining `python.exe` mid-copy —
/// so both flags need to be here, not just whichever one today's uv happens
/// to require.
pub fn create_venv(
    uv: &Path,
    venv: &Path,
    runtime_dir: &Path,
    interpreter: Option<&Path>,
) -> CommandSpec {
    let venv = venv.to_string_lossy().into_owned();
    let python = match interpreter {
        Some(path) => path.to_string_lossy().into_owned(),
        None => PYTHON_VERSION.to_string(),
    };
    spec(
        uv,
        &["venv", &venv, "--python", &python, "--clear", "--force"],
        Some(runtime_dir),
    )
}

/// Install a requirements file into the venv. `index_url`, when set, points uv
/// at a package index other than PyPI — a corporate mirror, typically —
/// since uv reads neither `pip.conf` nor `PIP_INDEX_URL`, the settings that let
/// the old `pip install --user` path work behind a firewall.
pub fn install_requirements(
    uv: &Path,
    venv_python: &Path,
    requirements: &Path,
    index_url: Option<&str>,
) -> CommandSpec {
    let python = venv_python.to_string_lossy().into_owned();
    let req = requirements.to_string_lossy().into_owned();
    let mut cmd = spec(
        uv,
        &["pip", "install", "--python", &python, "-r", &req],
        None,
    );
    if let Some(url) = index_url {
        cmd.env.insert("UV_INDEX_URL".to_string(), url.to_string());
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uv() -> PathBuf { PathBuf::from("/opt/aiterm/uv") }

    #[test]
    fn python_install_pins_the_version_and_redirects_the_install_dir() {
        let spec = install_python(&uv(), &PathBuf::from("/data/python-runtimes"));

        assert_eq!(spec.program, uv());
        assert_eq!(spec.args, vec!["python", "install", PYTHON_VERSION]);
        assert_eq!(
            spec.env.get("UV_PYTHON_INSTALL_DIR").map(String::as_str),
            Some("/data/python-runtimes")
        );
    }

    #[test]
    fn venv_creation_uses_the_managed_interpreter_by_default() {
        let spec = create_venv(&uv(), &PathBuf::from("/data/python-env"), &PathBuf::from("/data/rt"), None);

        assert_eq!(spec.args, vec!["venv", "/data/python-env", "--python", PYTHON_VERSION, "--clear", "--force"]);
        // Without this, uv can't find the interpreter it just installed into the
        // managed runtime dir — and dropping it silently left all six tests green.
        assert_eq!(
            spec.env.get("UV_PYTHON_INSTALL_DIR").map(String::as_str),
            Some("/data/rt")
        );
    }

    #[test]
    fn venv_creation_honours_a_user_specified_interpreter() {
        let spec = create_venv(
            &uv(),
            &PathBuf::from("/data/python-env"),
            &PathBuf::from("/data/rt"),
            Some(&PathBuf::from("/usr/local/bin/python3.12")),
        );

        assert_eq!(
            spec.args,
            vec!["venv", "/data/python-env", "--python", "/usr/local/bin/python3.12", "--clear", "--force"]
        );
    }

    #[test]
    fn venv_creation_always_clears_a_pre_existing_target() {
        // uv refuses outright ("A virtual environment already exists at: ...")
        // if the directory is already there — including the empty shell an
        // interrupted install leaves behind. Without --clear, every retry after
        // such an interruption fails the same unhelpful way forever.
        let spec = create_venv(&uv(), &PathBuf::from("/data/python-env"), &PathBuf::from("/data/rt"), None);
        assert!(spec.args.contains(&"--clear".to_string()));
    }

    #[test]
    fn venv_creation_always_forces_past_a_non_venv_directory_too() {
        // --clear alone still succeeds against a stray non-venv directory (a
        // different interrupted-install shape than the "already a venv" case
        // --clear itself covers), but only with a deprecation warning that uv
        // has said becomes a hard error in a future release — --force is what
        // silences that and keeps the guarantee once uv makes the change.
        let spec = create_venv(&uv(), &PathBuf::from("/data/python-env"), &PathBuf::from("/data/rt"), None);
        assert!(spec.args.contains(&"--force".to_string()));
    }

    #[test]
    fn pip_install_targets_the_venv_interpreter_not_the_system_one() {
        let spec = install_requirements(
            &uv(),
            &PathBuf::from("/data/python-env/bin/python"),
            &PathBuf::from("/tools/MarkItDown/requirements.txt"),
            None,
        );

        assert_eq!(
            spec.args,
            vec![
                "pip",
                "install",
                "--python",
                "/data/python-env/bin/python",
                "-r",
                "/tools/MarkItDown/requirements.txt",
            ]
        );
        // Deliberately absent here: --python already names the exact interpreter,
        // so this call has no business pointing uv at the managed runtime dir.
        assert!(!spec.env.contains_key("UV_PYTHON_INSTALL_DIR"));
    }

    #[test]
    fn pip_install_without_an_index_url_does_not_set_one() {
        let spec = install_requirements(
            &uv(),
            &PathBuf::from("/env/bin/python"),
            &PathBuf::from("/r.txt"),
            None,
        );
        assert!(!spec.env.contains_key("UV_INDEX_URL"));
    }

    #[test]
    fn pip_install_with_an_index_url_passes_it_to_uv() {
        // Verified against the bundled uv 0.11.19: pointed at a bogus host via
        // UV_INDEX_URL, it tried to fetch from that host rather than pypi.org —
        // this is the env var uv actually reads, not UV_INDEX or
        // UV_DEFAULT_INDEX.
        let spec = install_requirements(
            &uv(),
            &PathBuf::from("/env/bin/python"),
            &PathBuf::from("/r.txt"),
            Some("https://pypi.mycompany.com/simple"),
        );
        assert_eq!(
            spec.env.get("UV_INDEX_URL").map(String::as_str),
            Some("https://pypi.mycompany.com/simple")
        );
    }

    #[test]
    fn every_spec_disables_uvs_progress_animation() {
        // The log panel shows plain lines; uv's spinner would render as noise.
        let specs = [
            install_python(&uv(), &PathBuf::from("/rt")),
            create_venv(&uv(), &PathBuf::from("/env"), &PathBuf::from("/rt"), None),
            install_requirements(&uv(), &PathBuf::from("/env/bin/python"), &PathBuf::from("/r.txt"), None),
        ];
        for spec in specs {
            assert_eq!(spec.env.get("UV_NO_PROGRESS").map(String::as_str), Some("1"));
        }
    }

    #[test]
    fn python_version_is_pinned_to_the_intended_release() {
        // The other tests compare against PYTHON_VERSION itself, so they stay
        // green no matter what it's changed to. The whole point of pinning is
        // that this value doesn't drift, so assert the literal — MarkItDown
        // needs >= 3.10, and moving off 3.12 should be a deliberate edit that
        // updates this test too.
        assert_eq!(PYTHON_VERSION, "3.12");
    }
}
