//! 依副檔名把清單項目分類，用於「全部／資料夾／圖片／文件…」篩選籤。
//! 只看名稱，不讀取檔案內容。

use crate::entry::Entry;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    All = 0,
    Folder = 1,
    Image = 2,
    Document = 3,
    Media = 4,
    Archive = 5,
    Other = 6,
}

pub const COUNT: usize = 7;

pub const ALL: [Category; COUNT] =
    [Category::All, Category::Folder, Category::Image, Category::Document, Category::Media, Category::Archive, Category::Other];

const IMAGE: &[&str] = &[
    "jpg", "jpeg", "jpe", "jfif", "png", "gif", "bmp", "dib", "webp", "heic", "heif", "avif", "tif", "tiff", "ico", "svg", "raw", "cr2",
    "cr3", "nef", "arw", "dng", "psd",
];
const DOCUMENT: &[&str] = &[
    "pdf", "doc", "docx", "xls", "xlsx", "xlsm", "ppt", "pptx", "odt", "ods", "odp", "rtf", "txt", "md", "csv", "tsv", "json", "xml",
    "html", "htm", "log", "epub", "one", "pages", "numbers", "key",
];
const MEDIA: &[&str] = &[
    "mp3", "wav", "flac", "aac", "m4a", "ogg", "opus", "wma", "aif", "aiff", "mid", "midi", "mp4", "mkv", "mov", "avi", "wmv", "webm",
    "m4v", "mpg", "mpeg", "ts", "3gp",
];
const ARCHIVE: &[&str] = &["zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz", "zst", "iso", "cab", "lz", "lzma"];

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::All => "全部",
            Category::Folder => "資料夾",
            Category::Image => "圖片",
            Category::Document => "文件",
            Category::Media => "影音",
            Category::Archive => "壓縮檔",
            Category::Other => "其他",
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn of(entry: &Entry) -> Category {
        if entry.is_dir {
            return Category::Folder;
        }
        let ext = entry.extension();
        let ext = ext.as_str();
        if IMAGE.contains(&ext) {
            Category::Image
        } else if DOCUMENT.contains(&ext) {
            Category::Document
        } else if MEDIA.contains(&ext) {
            Category::Media
        } else if ARCHIVE.contains(&ext) {
            Category::Archive
        } else {
            Category::Other
        }
    }

    pub fn matches(self, entry: &Entry) -> bool {
        self == Category::All || Category::of(entry) == self
    }
}

/// 清單摘要：各分類數量、資料夾與檔案數、檔案總大小。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub counts: [usize; COUNT],
    pub folders: usize,
    pub files: usize,
    pub bytes: u64,
}

impl Summary {
    pub fn of(entries: &[Entry]) -> Summary {
        let mut s = Summary::default();
        for e in entries {
            s.add(e);
        }
        s
    }

    pub fn add(&mut self, e: &Entry) {
        self.counts[Category::All.index()] += 1;
        self.counts[Category::of(e).index()] += 1;
        if e.is_dir {
            self.folders += 1;
        } else {
            self.files += 1;
            self.bytes = self.bytes.saturating_add(e.size);
        }
    }
}

/// 套用篩選後要顯示的列：None 表示全部顯示（不額外配置記憶體）。
pub fn filter_rows(entries: &[Entry], category: Category) -> Option<Vec<u32>> {
    if category == Category::All {
        return None;
    }
    Some(entries.iter().enumerate().filter(|(_, e)| category.matches(e)).map(|(i, _)| i as u32).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::sample;

    #[test]
    fn classify() {
        assert_eq!(Category::of(&sample("src", true, 0, 0)), Category::Folder);
        assert_eq!(Category::of(&sample("a.PNG", false, 0, 0)), Category::Image);
        assert_eq!(Category::of(&sample("報告.pdf", false, 0, 0)), Category::Document);
        assert_eq!(Category::of(&sample("song.flac", false, 0, 0)), Category::Media);
        assert_eq!(Category::of(&sample("x.7z", false, 0, 0)), Category::Archive);
        assert_eq!(Category::of(&sample("app.exe", false, 0, 0)), Category::Other);
        assert_eq!(Category::of(&sample("Makefile", false, 0, 0)), Category::Other);
    }

    #[test]
    fn summary_and_filter() {
        let v = vec![
            sample("docs", true, 0, 0),
            sample("a.jpg", false, 100, 0),
            sample("b.png", false, 50, 0),
            sample("c.txt", false, 7, 0),
        ];
        let s = Summary::of(&v);
        assert_eq!(s.counts[Category::All.index()], 4);
        assert_eq!(s.counts[Category::Image.index()], 2);
        assert_eq!(s.counts[Category::Folder.index()], 1);
        assert_eq!((s.folders, s.files, s.bytes), (1, 3, 157));
        assert_eq!(filter_rows(&v, Category::All), None);
        assert_eq!(filter_rows(&v, Category::Image), Some(vec![1, 2]));
        assert_eq!(filter_rows(&v, Category::Media), Some(vec![]));
    }
}
