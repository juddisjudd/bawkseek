use std::collections::HashMap;

use soulseek_rs::SharedDirectory;

use super::group::FileHit;
use crate::format;

const ATTR_BITRATE: u32 = 0;
const ATTR_DURATION: u32 = 1;
const ATTR_VBR: u32 = 2;
const ATTR_SAMPLE_RATE: u32 = 4;
const ATTR_BIT_DEPTH: u32 = 5;

#[derive(Clone, Debug)]
pub struct Node {
    pub name: String,
    pub path: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub depth: usize,
    pub files: Vec<FileHit>,
    pub total_files: usize,
    pub total_size: u64,
}

/// A user's shares as a folder tree; listings only name folders that hold files, so parents are filled in.
#[derive(Clone, Debug, Default)]
pub struct Listing {
    pub nodes: Vec<Node>,
    pub roots: Vec<usize>,
    pub files: usize,
    pub size: u64,
}

impl Listing {
    pub fn build(dirs: Vec<SharedDirectory>) -> Self {
        let mut listing = Listing::default();
        let mut index: HashMap<String, usize> = HashMap::new();

        for dir in dirs {
            let ix = listing.node_for(&dir.name, &mut index);
            let files: Vec<FileHit> = dir
                .files
                .into_iter()
                .map(|entry| {
                    let attr = |code| entry.attribute(code).filter(|value| *value > 0);
                    FileHit {
                        filename: format!("{}\\{}", dir.name, entry.name),
                        ext: format::extension(&entry.name),
                        size: entry.size,
                        bitrate: attr(ATTR_BITRATE),
                        duration: attr(ATTR_DURATION),
                        vbr: attr(ATTR_VBR).is_some(),
                        sample_rate: attr(ATTR_SAMPLE_RATE),
                        bit_depth: attr(ATTR_BIT_DEPTH),
                        name: entry.name,
                    }
                })
                .collect();
            listing.nodes[ix].files.extend(files);
        }

        for node in &mut listing.nodes {
            node.files
                .sort_by_cached_key(|file| file.name.to_lowercase());
        }
        let mut order: Vec<usize> = (0..listing.nodes.len()).collect();
        order.sort_by_key(|ix| std::cmp::Reverse(listing.nodes[*ix].depth));
        for ix in order {
            let own_files = listing.nodes[ix].files.len();
            let own_size: u64 = listing.nodes[ix].files.iter().map(|file| file.size).sum();
            listing.nodes[ix].total_files += own_files;
            listing.nodes[ix].total_size += own_size;
            if let Some(parent) = listing.nodes[ix].parent {
                let (files, size) = (listing.nodes[ix].total_files, listing.nodes[ix].total_size);
                listing.nodes[parent].total_files += files;
                listing.nodes[parent].total_size += size;
            }
        }
        let names: Vec<String> = listing
            .nodes
            .iter()
            .map(|node| node.name.to_lowercase())
            .collect();
        for node in &mut listing.nodes {
            node.children.sort_by(|a, b| names[*a].cmp(&names[*b]));
        }
        listing.roots.sort_by(|a, b| names[*a].cmp(&names[*b]));
        listing.files = listing
            .roots
            .iter()
            .map(|ix| listing.nodes[*ix].total_files)
            .sum();
        listing.size = listing
            .roots
            .iter()
            .map(|ix| listing.nodes[*ix].total_size)
            .sum();
        listing
    }

    fn node_for(&mut self, path: &str, index: &mut HashMap<String, usize>) -> usize {
        if let Some(ix) = index.get(path) {
            return *ix;
        }
        let (parent_path, name) = format::split_path(path);
        let parent = (!parent_path.is_empty()).then(|| self.node_for(parent_path, index));
        let ix = self.nodes.len();
        self.nodes.push(Node {
            name: name.to_string(),
            path: path.to_string(),
            parent,
            children: Vec::new(),
            depth: parent.map_or(0, |parent| self.nodes[parent].depth + 1),
            files: Vec::new(),
            total_files: 0,
            total_size: 0,
        });
        match parent {
            Some(parent) => self.nodes[parent].children.push(ix),
            None => self.roots.push(ix),
        }
        index.insert(path.to_string(), ix);
        ix
    }

    /// Every file under `ix`, with its folder path relative to `ix`, for downloading a whole subtree.
    pub fn files_under(&self, ix: usize) -> Vec<(&FileHit, String)> {
        let mut out = Vec::new();
        let mut stack = vec![(ix, String::new())];
        while let Some((node, relative)) = stack.pop() {
            for file in &self.nodes[node].files {
                out.push((file, relative.clone()));
            }
            for child in self.nodes[node].children.iter().rev() {
                let name = &self.nodes[*child].name;
                let path = if relative.is_empty() {
                    name.clone()
                } else {
                    format!("{relative}\\{name}")
                };
                stack.push((*child, path));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use soulseek_rs::SharedFileEntry;

    use super::*;

    fn dir(name: &str, files: &[(&str, u64)]) -> SharedDirectory {
        SharedDirectory {
            name: name.into(),
            files: files
                .iter()
                .map(|(name, size)| SharedFileEntry {
                    name: (*name).into(),
                    size: *size,
                    attributes: vec![(0, 320), (1, 200)],
                })
                .collect(),
        }
    }

    #[test]
    fn fills_in_missing_parents_and_totals() {
        let listing = Listing::build(vec![
            dir("Music\\B Artist\\Album", &[("2.mp3", 20), ("1.mp3", 10)]),
            dir("Music\\a artist", &[("x.mp3", 5)]),
            dir("Other", &[("readme.txt", 1)]),
        ]);

        assert_eq!(listing.files, 4);
        assert_eq!(listing.size, 36);
        let roots: Vec<&str> = listing
            .roots
            .iter()
            .map(|ix| listing.nodes[*ix].name.as_str())
            .collect();
        assert_eq!(roots, vec!["Music", "Other"]);

        let music = &listing.nodes[listing.roots[0]];
        assert_eq!(music.total_files, 3);
        assert_eq!(music.total_size, 35);
        let children: Vec<&str> = music
            .children
            .iter()
            .map(|ix| listing.nodes[*ix].name.as_str())
            .collect();
        assert_eq!(children, vec!["a artist", "B Artist"]);

        let album = listing
            .nodes
            .iter()
            .find(|node| node.name == "Album")
            .unwrap();
        assert_eq!(album.depth, 2);
        assert_eq!(album.files[0].name, "1.mp3");
        assert_eq!(album.files[0].filename, "Music\\B Artist\\Album\\1.mp3");
        assert_eq!(album.files[0].bitrate, Some(320));
    }

    #[test]
    fn lists_subtree_files_with_relative_folders() {
        let listing = Listing::build(vec![
            dir("Music\\Album", &[("cover.jpg", 1)]),
            dir("Music\\Album\\CD1", &[("1.flac", 2)]),
            dir("Music\\Album\\CD2", &[("1.flac", 3)]),
        ]);
        let album = listing
            .nodes
            .iter()
            .position(|node| node.name == "Album")
            .unwrap();
        let files: Vec<(String, String)> = listing
            .files_under(album)
            .into_iter()
            .map(|(file, relative)| (file.filename.clone(), relative))
            .collect();
        assert_eq!(
            files,
            vec![
                ("Music\\Album\\cover.jpg".into(), String::new()),
                ("Music\\Album\\CD1\\1.flac".into(), "CD1".into()),
                ("Music\\Album\\CD2\\1.flac".into(), "CD2".into()),
            ]
        );
    }
}
