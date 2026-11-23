//! 檔案清單的一列。

use crate::paths;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    /// 所在資料夾；同一資料夾的項目共用同一份字串以節省記憶體。
    pub dir: Arc<str>,
    pub is_dir: bool,
    pub size: u64,
    /// FILETIME（100 奈秒，UTC，自 1601 年起）。
    pub modified: u64,
    /// 重新解析點（符號連結、junction、部分雲端位置）。
    pub is_link: bool,
}

impl Entry {
    pub fn full_path(&self) -> String {
        paths::join(&self.dir, &self.name)
    }

    pub fn size(&self) -> Option<u64> {
        if self.is_dir {
            None
        } else {
            Some(self.size)
        }
    }

    /// 小寫副檔名（不含句點）；資料夾為空字串。
    pub fn extension(&self) -> String {
        if self.is_dir {
            String::new()
        } else {
            paths::extension_lower(&self.name)
        }
    }

    /// 顯示名稱：關閉副檔名時去掉最後一段；像 ".gitignore" 這種只有副檔名的名稱維持原樣。
    pub fn display_name(&self, show_extensions: bool) -> &str {
        if self.is_dir || show_extensions {
            return &self.name;
        }
        match self.name.rfind('.') {
            Some(i) if i > 0 => &self.name[..i],
            _ => &self.name,
        }
    }

    /// 類型欄：「連結」、「資料夾」、「PNG 檔案」、「檔案」。
    pub fn type_label(&self) -> String {
        if self.is_link {
            tr!("連結", "Link").to_string()
        } else if self.is_dir {
            tr!("資料夾", "Folder").to_string()
        } else {
            let ext = self.extension();
            if ext.is_empty() {
                tr!("檔案", "File").to_string()
            } else {
                trf!("{} 檔案", "{} File", ext.to_uppercase())
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn sample(name: &str, is_dir: bool, size: u64, modified: u64) -> Entry {
    Entry { name: name.into(), dir: Arc::from("C:\\t"), is_dir, size, modified, is_link: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        let f = sample("README.md", false, 10, 0);
        assert_eq!(f.type_label(), "MD 檔案");
        assert_eq!(f.display_name(false), "README");
        assert_eq!(f.display_name(true), "README.md");
        assert_eq!(f.full_path(), "C:\\t\\README.md");
        let g = sample(".gitignore", false, 1, 0);
        assert_eq!(g.display_name(false), ".gitignore");
        assert_eq!(g.type_label(), "GITIGNORE 檔案");
        let d = sample("src", true, 0, 0);
        assert_eq!(d.type_label(), "資料夾");
        assert_eq!(d.size(), None);
        let n = sample("Makefile", false, 0, 0);
        assert_eq!(n.type_label(), "檔案");
    }
}
