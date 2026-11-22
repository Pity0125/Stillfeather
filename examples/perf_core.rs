//! Stillfeather 核心邏輯的效能量測（不碰 Win32，任何平台都能跑）。
//!
//! 執行：`cargo run --release --example perf_core`
//! 每一項重複數次取中位數；資料以固定亂數產生，每次結果可比較。
//! 「目標」是以使用者感覺不到延遲為準：一次操作 ≤ 16 ms（一個畫格）算順，≤ 100 ms 算即時。

use feather_core::category::{filter_rows, Category, Summary};
use feather_core::entry::Entry;
use feather_core::folderviews::FolderViews;
use feather_core::lru::ByteLru;
use feather_core::namematch::Needle;
use feather_core::quicklook::{self, Lang};
use feather_core::settings::Settings;
use feather_core::sorting::{self, natural_cmp};
use feather_core::view::{self, ViewMode};
use feather_core::{json, typeahead};
use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// 像真實資料夾的檔名：相機照片、文件副本、中文名稱、UUID 影片、編號檔、資料夾。
fn entries(n: usize, seed: u64) -> Vec<Entry> {
    let mut r = Rng(seed);
    let dir: Arc<str> = Arc::from("C:\\Users\\pity0\\Pictures\\很多檔案");
    let exts = ["jpg", "png", "mp4", "docx", "pdf", "txt", "rs", "zip", "mp3", "xlsx", "heic", "md"];
    (0..n)
        .map(|i| {
            let ext = exts[r.below(exts.len() as u64) as usize];
            let name = match r.below(7) {
                0 => format!("IMG_2024{:02}{:02}_{:06}.{ext}", 1 + r.below(12), 1 + r.below(28), r.below(1_000_000)),
                1 => format!("文件 ({}).{ext}", r.below(500)),
                2 => format!("報告_第{}版_{}.{ext}", r.below(30), i),
                3 => format!("Minecraft_{:08x}-{:04x}-4{:03x}-9{:03x}-{:012x}.{ext}", r.next() as u32, r.below(65536), r.below(4096), r.below(4096), r.next() & 0xFFFF_FFFF_FFFF),
                4 => format!("file{}.{ext}", r.below(100_000)),
                5 => format!("Résumé – Final v{}.{ext}", r.below(20)),
                _ => format!("Folder {}", r.below(10_000)),
            };
            let is_dir = !name.contains('.');
            Entry { name, dir: dir.clone(), is_dir, size: r.below(4_000_000_000), modified: 133_000_000_000_000_000 + r.below(10_000_000_000_000), is_link: false }
        })
        .collect()
}

/// 每次先準備一份新資料（不計時），再量 `f` 本身；回傳中位數（毫秒）。
fn time_ms_with<T>(runs: usize, mut setup: impl FnMut() -> T, mut f: impl FnMut(&mut T)) -> f64 {
    let mut t: Vec<f64> = (0..runs)
        .map(|_| {
            let mut data = setup();
            let s = Instant::now();
            f(&mut data);
            let ms = s.elapsed().as_secs_f64() * 1000.0;
            black_box(data);
            ms
        })
        .collect();
    t.sort_by(|a, b| a.partial_cmp(b).unwrap());
    t[t.len() / 2]
}

/// 執行 `f` 數次，回傳中位數（毫秒）。
fn time_ms(runs: usize, mut f: impl FnMut()) -> f64 {
    let mut t: Vec<f64> = (0..runs)
        .map(|_| {
            let s = Instant::now();
            f();
            s.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    t.sort_by(|a, b| a.partial_cmp(b).unwrap());
    t[t.len() / 2]
}

fn verdict(ms: f64, budget: f64) -> &'static str {
    if ms <= budget {
        "OK"
    } else if ms <= budget * 3.0 {
        "偏慢"
    } else {
        "慢"
    }
}

fn row(name: &str, ms: f64, budget: f64) {
    println!("{:<44} {:>10.3} ms   目標 ≤ {:>6} ms   {}", name, ms, budget, verdict(ms, budget));
}

fn code_sample(lang: Lang, bytes: usize) -> String {
    let unit = match lang {
        Lang::Rust => "/// 說明文件\npub fn compute(items: &[u32], factor: f64) -> Option<String> {\n    let total: u64 = items.iter().map(|x| *x as u64).sum(); // 加總\n    if total > 0x1F_FFFF { return None; }\n    Some(format!(\"{} {:.2}\", total, factor * 1.5e3))\n}\n#[derive(Clone, Debug)]\nstruct Point { x: i32, y: i32 }\nimpl Point { const ORIGIN: Point = Point { x: 0, y: 0 }; }\n",
        Lang::JavaScript => "// comment\nexport async function load(url, { retries = 3 } = {}) {\n  const res = await fetch(`${url}?t=${Date.now()}`);\n  if (!res.ok) throw new Error('failed: ' + res.status);\n  return /^[a-z]+$/i.test(res.statusText) ? res.json() : null;\n}\nclass Store extends Base { #items = []; get size() { return this.#items.length; } }\n",
        Lang::Json => "{\n  \"name\": \"feather\",\n  \"version\": \"0.1.0\",\n  \"list\": [1, 2.5, -3e4, true, false, null],\n  \"nested\": { \"中文\": \"值\", \"escape\": \"\\n\\t\\u4e2d\" }\n},\n",
        Lang::Markdown => "# 標題\n\n一段 **粗體** 與 `程式碼` 還有 [連結](https://example.com)。\n\n- 清單一\n- 清單二\n\n```rust\nfn main() {}\n```\n\n> 引用文字\n",
        Lang::Xml => "<?xml version=\"1.0\"?>\n<Project Sdk=\"Microsoft.NET.Sdk\">\n  <!-- 註解 -->\n  <PropertyGroup Condition=\"'$(Configuration)'=='Release'\">\n    <OutputType>WinExe</OutputType>\n  </PropertyGroup>\n</Project>\n",
        Lang::Python => "def main(argv: list[str]) -> int:\n    \"\"\"說明\"\"\"\n    total = sum(int(x) for x in argv if x.isdigit())  # 註解\n    print(f\"total={total!r}\")\n    return 0 if total else 1\n\nclass A(B):\n    @property\n    def x(self): return 0x10\n",
        _ => "2026-11-21 22:05:36.171 [32668] 啟動 Stillfeather 0.1.0 一般的記錄檔內容，沒有任何語法上色，只是很長的純文字。\n",
    };
    let mut s = String::with_capacity(bytes + unit.len());
    while s.len() < bytes {
        s.push_str(unit);
    }
    s
}

fn main() {
    println!("Stillfeather 核心效能量測（{}，{}）", std::env::consts::OS, if cfg!(debug_assertions) { "debug：數字不準，請加 --release" } else { "release" });
    println!("{}", "-".repeat(92));

    // ---------------------------------------------------------------- 排序
    for &n in &[1_000usize, 10_000, 100_000] {
        let base = entries(n, 42);
        let budget = if n <= 10_000 { 16.0 } else { 100.0 };
        for (col, label) in [(sorting::COL_NAME, "名稱"), (sorting::COL_SIZE, "大小"), (sorting::COL_TYPE, "類型"), (sorting::COL_MODIFIED, "修改日期")] {
            let ms = time_ms_with(5, || base.clone(), |v| sorting::sort_entries(v, col, false));
            row(&format!("排序 {n:>7} 項（依{label}）"), ms, budget);
        }
    }
    {
        let names: Vec<String> = entries(2_000, 7).into_iter().map(|e| e.name).collect();
        let pairs = names.len() * names.len();
        let ms = time_ms(3, || {
            for a in &names {
                for b in &names {
                    black_box(natural_cmp(a, b));
                }
            }
        });
        println!("{:<44} {:>10.1} ns/次", "自然排序比較 natural_cmp", ms * 1e6 / pairs as f64);
    }

    // ---------------------------------------------------------------- 類型篩選與統計
    {
        let base = entries(100_000, 3);
        row("類型統計 Summary 100,000 項", time_ms(5, || { black_box(Summary::of(&base)); }), 16.0);
        row("篩選「圖片」100,000 項", time_ms(5, || { black_box(filter_rows(&base, Category::Image)); }), 16.0);
        // 清單捲動：每一列要名稱、類型、大小（LVN_GETDISPINFO）
        row("清單顯示文字 100,000 列（整個捲過）", time_ms(3, || {
            for e in &base {
                black_box(e.display_name(false));
                black_box(e.type_label());
                black_box(feather_core::format::format_size(e.size()));
            }
        }), 100.0);
    }

    // ---------------------------------------------------------------- 搜尋檔名比對
    {
        let names: Vec<Vec<u16>> = entries(1_000_000, 9).into_iter().map(|e| e.name.encode_utf16().collect()).collect();
        for q in ["minecraft", "報告", "zzzz-not-found"] {
            let needle = Needle::new(q).unwrap();
            let ms = time_ms(3, || {
                black_box(names.iter().filter(|n| needle.matches(n)).count());
            });
            row(&format!("搜尋比對 1,000,000 個檔名「{q}」"), ms, 100.0);
        }
    }

    // ---------------------------------------------------------------- 打字跳轉
    {
        let base = entries(100_000, 5);
        let ms = time_ms(5, || {
            black_box(typeahead::find(base.len(), Some(0), "qqqq-沒有", |i| base.get(i).map(|e| e.display_name(true).to_string())));
        });
        row("打字跳轉 100,000 項（找不到，整個掃過）", ms, 16.0);
    }

    // ---------------------------------------------------------------- 快速預覽：文字解碼與上色（1 MiB 上限）
    for (lang, label) in [
        (Lang::Rust, "Rust"),
        (Lang::JavaScript, "JavaScript"),
        (Lang::Json, "JSON"),
        (Lang::Markdown, "Markdown"),
        (Lang::Xml, "XML"),
        (Lang::Python, "Python"),
        (Lang::Plain, "純文字"),
    ] {
        let text = code_sample(lang, 1024 * 1024);
        let bytes = text.clone().into_bytes();
        let ms = time_ms(5, || {
            if let quicklook::Decoded::Text(t) = quicklook::decode(&bytes) {
                black_box(quicklook::highlight(&t, lang));
            }
        });
        row(&format!("預覽 1 MiB {label}（解碼＋上色）"), ms, 100.0);
    }
    {
        // 壓縮過的 JavaScript：一行很長
        let line = "var a=function(b){return b+1};".repeat(40_000);
        let ms = time_ms(5, || {
            black_box(quicklook::highlight(&line, Lang::JavaScript));
        });
        row("預覽 1.2 MB 單行壓縮 JavaScript", ms, 100.0);
        let utf16: Vec<u8> = code_sample(Lang::Plain, 1024 * 1024).encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        let ms = time_ms(5, || {
            black_box(quicklook::decode(&utf16));
        });
        row("預覽解碼 2 MiB UTF-16（無 BOM）", ms, 100.0);
    }

    // ---------------------------------------------------------------- 設定檔與資料夾檢視記錄
    {
        let s = Settings { favorites: (0..50).map(|i| format!("C:\\Users\\pity0\\常用\\資料夾 {i}")).collect(), ..Default::default() };
        let text = s.to_json();
        row("設定檔讀取（50 個常用位置）", time_ms(20, || { black_box(Settings::from_json(&text).unwrap()); }), 16.0);
        let mut v = FolderViews::default();
        for i in 0..5_000 {
            v.set(&format!("C:\\資料\\專案 {i}\\子資料夾"), ViewMode::Large, ViewMode::Details);
        }
        let text = v.to_json();
        row("資料夾檢視記錄讀取（5,000 筆，啟動時）", time_ms(5, || { black_box(FolderViews::from_json(&text).unwrap()); }), 50.0);
        row("資料夾檢視記錄寫入（5,000 筆）", time_ms(5, || { black_box(v.to_json()); }), 50.0);
        row("記錄已滿時新增一筆（淘汰最舊）", time_ms(20, || {
            let mut w = v.clone();
            w.set("C:\\新的資料夾", ViewMode::Tiles, ViewMode::Details);
            black_box(w);
        }), 16.0);
        let big = format!("[{}]", (0..60_000).map(|i| format!("{{\"k\":\"值{i}\",\"n\":{i}.5,\"b\":true}}")).collect::<Vec<_>>().join(","));
        row(&format!("JSON 解析 {:.1} MB", big.len() as f64 / 1e6), time_ms(5, || { black_box(json::parse(&big).is_ok()); }), 100.0);
    }

    // ---------------------------------------------------------------- 大圖示縮放
    {
        let src: Vec<u32> = (0..256 * 256).map(|i| 0xFF00_0000 | (i as u32 * 2654435761) & 0x00FF_FFFF).collect();
        let ms = time_ms(20, || {
            black_box(view::resample(&src, 256, 96));
        });
        row("圖示縮放 256→96（每個圖示）", ms, 2.0);
        let small: Vec<u32> = (0..48 * 48).map(|_| 0xFF33_6699).collect();
        let ms = time_ms(20, || {
            black_box(view::center_into(&small, 48, 256));
        });
        row("小圖示置中到 256 畫布", ms, 2.0);
    }

    // ---------------------------------------------------------------- 預覽快取
    {
        let mut c: ByteLru<String, Vec<u8>> = ByteLru::new(32 << 20, 24);
        for i in 0..24 {
            c.put(format!("C:\\影片\\{i}.mp4"), vec![0u8; 1 << 20], 1 << 20);
        }
        let ms = time_ms(20, || {
            for i in 0..24 {
                black_box(c.get(&format!("C:\\影片\\{i}.mp4")));
            }
        });
        row("預覽快取取出 24 次（每次 1 MiB）", ms, 16.0);
    }
    println!("{}", "-".repeat(92));
}
