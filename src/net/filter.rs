use super::group::{FileHit, FolderHit, is_lossless};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Quality {
    #[default]
    Any,
    AtLeast(u32),
    Lossless,
    HiRes,
}

impl Quality {
    pub const CHOICES: [Quality; 7] = [
        Quality::Any,
        Quality::AtLeast(128),
        Quality::AtLeast(192),
        Quality::AtLeast(256),
        Quality::AtLeast(320),
        Quality::Lossless,
        Quality::HiRes,
    ];

    pub fn label(self) -> String {
        match self {
            Quality::Any => "any quality".into(),
            Quality::AtLeast(kbps) => format!("≥ {kbps} kbps"),
            Quality::Lossless => "lossless".into(),
            Quality::HiRes => "24-bit or hi-res".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cmp {
    Lt,
    Le,
    Eq,
    Ge,
    Gt,
}

impl Cmp {
    fn holds(self, value: u32, limit: u32) -> bool {
        match self {
            Cmp::Lt => value < limit,
            Cmp::Le => value <= limit,
            Cmp::Eq => value == limit,
            Cmp::Ge => value >= limit,
            Cmp::Gt => value > limit,
        }
    }
}

/// The search filter box and its chips: words match folders, while formats and quality pick files.
#[derive(Debug, Default, PartialEq)]
pub struct Filter {
    words: Vec<String>,
    excluded: Vec<String>,
    formats: Vec<String>,
    bitrates: Vec<(Cmp, u32)>,
    quality: Quality,
}

impl Filter {
    pub fn new(text: &str, formats: &[String], quality: Quality) -> Self {
        let mut filter = Filter {
            formats: formats.to_vec(),
            quality,
            ..Filter::default()
        };
        let text = text.to_lowercase();
        let mut tokens = text.split_whitespace().peekable();
        while let Some(token) = tokens.next() {
            let joined;
            let token = match tokens.peek() {
                Some(next) if comparison(token).is_some_and(|(_, rest)| rest.is_empty()) => {
                    joined = format!("{token}{next}");
                    if bitrate(&joined).is_some() {
                        tokens.next();
                        joined.as_str()
                    } else {
                        token
                    }
                }
                _ => token,
            };
            if let Some(limit) = bitrate(token) {
                filter.bitrates.push(limit);
            } else if let Some(ext) = token
                .strip_prefix('.')
                .filter(|ext| !ext.is_empty() && ext.chars().all(char::is_alphanumeric))
            {
                filter.formats.push(ext.to_string());
            } else if let Some(word) = token.strip_prefix('-').filter(|word| !word.is_empty()) {
                filter.excluded.push(word.to_string());
            } else {
                filter.words.push(token.to_string());
            }
        }
        filter
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty() && self.excluded.is_empty() && !self.picks_files()
    }

    fn picks_files(&self) -> bool {
        !self.formats.is_empty() || !self.bitrates.is_empty() || self.quality != Quality::Any
    }

    pub fn file(&self, file: &FileHit) -> bool {
        if !self.formats.is_empty() && !self.formats.contains(&file.ext) {
            return false;
        }
        let kbps = kbps(file);
        let bitrates = self
            .bitrates
            .iter()
            .all(|(cmp, limit)| kbps.is_some_and(|kbps| cmp.holds(kbps, *limit)));
        bitrates
            && match self.quality {
                Quality::Any => true,
                Quality::AtLeast(limit) => kbps.is_some_and(|kbps| kbps >= limit),
                Quality::Lossless => is_lossless(&file.ext),
                Quality::HiRes => {
                    is_lossless(&file.ext)
                        && (file.bit_depth.is_some_and(|depth| depth >= 24)
                            || file.sample_rate.is_some_and(|rate| rate > 48_000))
                }
            }
    }

    /// The folder's files that pass, or `None` when the folder should be hidden.
    pub fn folder(&self, folder: &FolderHit) -> Option<Vec<usize>> {
        let all = || (0..folder.files.len()).collect();
        if self.is_empty() {
            return Some(all());
        }
        let haystack =
            format!("{} {} {}", folder.username, folder.folder, folder.summary()).to_lowercase();
        let mentions = |word: &str| {
            haystack.contains(word)
                || folder
                    .files
                    .iter()
                    .any(|file| file.name.to_lowercase().contains(word))
        };
        if !self.words.iter().all(|word| mentions(word))
            || self.excluded.iter().any(|word| mentions(word))
        {
            return None;
        }
        if !self.picks_files() {
            return Some(all());
        }
        let files: Vec<usize> = (0..folder.files.len())
            .filter(|ix| self.file(&folder.files[*ix]))
            .collect();
        (!files.is_empty()).then_some(files)
    }
}

fn comparison(token: &str) -> Option<(Cmp, &str)> {
    [
        (">=", Cmp::Ge),
        ("≥", Cmp::Ge),
        ("<=", Cmp::Le),
        ("≤", Cmp::Le),
        (">", Cmp::Gt),
        ("<", Cmp::Lt),
        ("=", Cmp::Eq),
    ]
    .into_iter()
    .find_map(|(prefix, cmp)| token.strip_prefix(prefix).map(|rest| (cmp, rest)))
}

fn bitrate(token: &str) -> Option<(Cmp, u32)> {
    let (cmp, rest) = comparison(token)?;
    let digits = rest
        .strip_suffix("kbps")
        .or_else(|| rest.strip_suffix("kb/s"))
        .or_else(|| rest.strip_suffix('k'))
        .unwrap_or(rest);
    Some((cmp, digits.parse().ok()?))
}

/// Lossless files often carry no bitrate, so their uncompressed rate stands in for it.
fn kbps(file: &FileHit) -> Option<u32> {
    file.bitrate.or_else(|| {
        is_lossless(&file.ext).then(|| match (file.sample_rate, file.bit_depth) {
            (Some(rate), Some(depth)) => rate * depth * 2 / 1000,
            _ => 1411,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(name: &str, bitrate: Option<u32>, depth: Option<u32>, rate: Option<u32>) -> FileHit {
        FileHit {
            filename: format!("music\\Album\\{name}"),
            name: name.into(),
            ext: name.rsplit('.').next().unwrap_or_default().into(),
            size: 1,
            bitrate,
            duration: None,
            vbr: false,
            sample_rate: rate,
            bit_depth: depth,
        }
    }

    fn folder(files: Vec<FileHit>) -> FolderHit {
        FolderHit {
            username: "ann".into(),
            folder: "music\\Album".into(),
            name: "Album".into(),
            files,
            size: 0,
            speed: 0,
            free: true,
            format: "mp3".into(),
            quality: "320".into(),
        }
    }

    #[test]
    fn reads_formats_and_bitrates_from_text() {
        let filter = Filter::new(".FLAC > 128 <=320kbps -live ≥192 album", &[], Quality::Any);
        assert_eq!(filter.formats, ["flac"]);
        assert_eq!(
            filter.bitrates,
            [(Cmp::Gt, 128), (Cmp::Le, 320), (Cmp::Ge, 192)]
        );
        assert_eq!(filter.excluded, ["live"]);
        assert_eq!(filter.words, ["album"]);
    }

    #[test]
    fn a_lone_sign_stays_a_word() {
        let filter = Filter::new("> live", &[], Quality::Any);
        assert_eq!(filter.words, [">", "live"]);
        assert!(filter.bitrates.is_empty());
    }

    #[test]
    fn bitrate_limits_pick_files() {
        let mp3 = hit("a.mp3", Some(320), None, None);
        let low = hit("b.mp3", Some(128), None, None);
        let flac = hit("c.flac", None, Some(16), Some(44_100));
        let cover = hit("cover.jpg", None, None, None);
        let over = Filter::new(">128", &[], Quality::Any);
        assert!(over.file(&mp3) && !over.file(&low) && over.file(&flac) && !over.file(&cover));
        let under = Filter::new("<=320", &[], Quality::Any);
        assert!(under.file(&mp3) && under.file(&low) && !under.file(&flac));
        let exact = Filter::new("=320", &[], Quality::Any);
        assert!(exact.file(&mp3) && !exact.file(&low));
    }

    #[test]
    fn quality_choices_pick_files() {
        let mp3 = hit("a.mp3", Some(320), None, None);
        let cd = hit("b.flac", None, Some(16), Some(44_100));
        let hires = hit("c.flac", None, Some(24), Some(96_000));
        let at_least = Filter::new("", &[], Quality::AtLeast(256));
        assert!(at_least.file(&mp3) && at_least.file(&cd));
        let lossless = Filter::new("", &[], Quality::Lossless);
        assert!(!lossless.file(&mp3) && lossless.file(&cd) && lossless.file(&hires));
        let hi = Filter::new("", &[], Quality::HiRes);
        assert!(!hi.file(&cd) && hi.file(&hires));
    }

    #[test]
    fn folders_show_only_the_files_that_pass() {
        let album = folder(vec![
            hit("01.flac", None, Some(16), Some(44_100)),
            hit("01.mp3", Some(320), None, None),
            hit("cover.jpg", None, None, None),
        ]);
        let chips = Filter::new("", &["mp3".into()], Quality::Any);
        assert_eq!(chips.folder(&album), Some(vec![1]));
        assert_eq!(Filter::new(".wav", &[], Quality::Any).folder(&album), None);
        assert_eq!(
            Filter::new("ann", &[], Quality::Any).folder(&album),
            Some(vec![0, 1, 2])
        );
        assert_eq!(
            Filter::new("-cover", &[], Quality::Any).folder(&album),
            None
        );
        assert_eq!(
            Filter::new("", &[], Quality::Any).folder(&album),
            Some(vec![0, 1, 2])
        );
    }
}
