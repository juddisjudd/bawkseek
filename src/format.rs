pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn count(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (ix, digit) in digits.chars().enumerate() {
        if ix > 0 && (digits.len() - ix).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

pub fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{} {}", count(n), if n == 1 { one } else { many })
}

pub fn speed(bytes_per_second: u64) -> String {
    format!("{}/s", bytes(bytes_per_second))
}

pub fn duration(seconds: u32) -> String {
    let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Soulseek paths use `\` whatever the sharer's OS, so split on both.
pub fn split_path(path: &str) -> (&str, &str) {
    match path.rfind(['\\', '/']) {
        Some(ix) => (&path[..ix], &path[ix + 1..]),
        None => ("", path),
    }
}

pub fn extension(name: &str) -> String {
    match name.rfind('.') {
        Some(ix) if ix + 1 < name.len() => name[ix + 1..].to_ascii_lowercase(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bytes() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1023), "1023 B");
        assert_eq!(bytes(1536), "1.5 KB");
        assert_eq!(bytes(250 * 1024 * 1024), "250 MB");
        assert_eq!(bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn formats_counts() {
        assert_eq!(count(7), "7");
        assert_eq!(count(1234), "1,234");
        assert_eq!(count(1234567), "1,234,567");
        assert_eq!(plural(1, "file", "files"), "1 file");
        assert_eq!(plural(2000, "file", "files"), "2,000 files");
    }

    #[test]
    fn formats_duration() {
        assert_eq!(duration(5), "0:05");
        assert_eq!(duration(245), "4:05");
        assert_eq!(duration(3725), "1:02:05");
    }

    #[test]
    fn splits_soulseek_paths() {
        assert_eq!(
            split_path("@@music\\Artist\\Album\\01 Track.flac"),
            ("@@music\\Artist\\Album", "01 Track.flac")
        );
        assert_eq!(split_path("/home/u/a.mp3"), ("/home/u", "a.mp3"));
        assert_eq!(split_path("loose.ogg"), ("", "loose.ogg"));
    }

    #[test]
    fn reads_extension() {
        assert_eq!(extension("01 Track.FLAC"), "flac");
        assert_eq!(extension("noext"), "");
        assert_eq!(extension("trailing."), "");
    }
}
