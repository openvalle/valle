//! A real child process for analyzer transport tests, using the host's built-in shell.

use std::path::Path;

#[derive(Default)]
pub struct AnalyzerStub<'a> {
    pub stdout: &'a str,
    pub stderr: &'a str,
    pub request_path: Option<&'a Path>,
    pub counter_path: Option<&'a Path>,
    pub delay_ms: u64,
    pub exit_code: i32,
}

impl AnalyzerStub<'_> {
    #[cfg(not(windows))]
    pub fn command(&self, directory: &Path) -> Vec<String> {
        let quote = |text: &str| format!("'{}'", text.replace('\'', "'\"'\"'"));
        let mut body = String::new();
        if let Some(path) = self.counter_path {
            body += &format!("printf 'x\\n' >> {}\n", quote(&path.to_string_lossy()));
        }
        body += &match self.request_path {
            Some(path) => format!("cat > {}\n", quote(&path.to_string_lossy())),
            None => "cat > /dev/null\n".to_owned(),
        };
        if self.delay_ms > 0 {
            body += &format!("sleep {}\n", self.delay_ms as f64 / 1000.0);
        }
        body += &format!(
            "printf '%s\\n' {}\nprintf '%s' {} >&2\nexit {}\n",
            quote(self.stdout),
            quote(self.stderr),
            self.exit_code
        );
        let script = directory.join("analyzer.sh");
        std::fs::write(&script, body).unwrap();
        vec!["sh".to_owned(), script.to_string_lossy().into_owned()]
    }

    #[cfg(windows)]
    pub fn command(&self, _directory: &Path) -> Vec<String> {
        let quote = |text: &str| format!("'{}'", text.replace('\'', "''"));
        let mut body = String::from(
            "$ErrorActionPreference = 'Stop'; \
             $utf8 = [Text.UTF8Encoding]::new($false); \
             [Console]::InputEncoding = $utf8; [Console]::OutputEncoding = $utf8; ",
        );
        if let Some(path) = self.counter_path {
            body += &format!(
                "[IO.File]::AppendAllText({}, \"x`n\", $utf8); ",
                quote(&path.to_string_lossy())
            );
        }
        body += "$request = [Console]::In.ReadToEnd(); ";
        if let Some(path) = self.request_path {
            body += &format!(
                "[IO.File]::WriteAllText({}, $request, $utf8); ",
                quote(&path.to_string_lossy())
            );
        }
        if self.delay_ms > 0 {
            body += &format!("Start-Sleep -Milliseconds {}; ", self.delay_ms);
        }
        body += &format!(
            "[Console]::Out.WriteLine({}); [Console]::Error.Write({}); exit {}",
            quote(self.stdout),
            quote(self.stderr),
            self.exit_code
        );
        vec![
            "powershell.exe".to_owned(),
            "-NoProfile".to_owned(),
            "-NonInteractive".to_owned(),
            "-Command".to_owned(),
            body,
        ]
    }
}
