//! 動畫的純邏輯：補間值、緩動曲線，以及清單／樹狀圖逐列的選取與懸停淡入淡出。
//! 時間一律由呼叫端傳入（毫秒），方便在任何平台測試；Windows 介面層再接上實際時鐘。

use std::collections::HashMap;
use std::hash::Hash;

/// 緩動曲線。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ease {
    /// 快速起步、平順停下（大多數過場）。
    Out,
    /// 兩端都平順（位置滑動）。
    InOut,
    /// 稍微超過再回彈（圖示彈出）。
    OutBack,
}

pub fn ease(kind: Ease, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match kind {
        Ease::Out => 1.0 - (1.0 - t).powi(3),
        Ease::InOut => {
            if t < 0.5 {
                4.0 * t * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
            }
        }
        Ease::OutBack => {
            let c1 = 1.70158;
            let c3 = c1 + 1.0;
            1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
        }
    }
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// 從目前值平順移到目標值的補間。動畫途中換目標時，從當下的值接續，不會跳動。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tween {
    from: f32,
    to: f32,
    start: f64,
    dur: f64,
    kind: Ease,
}

impl Tween {
    pub const fn new(value: f32) -> Tween {
        Tween { from: value, to: value, start: 0.0, dur: 0.0, kind: Ease::Out }
    }

    pub fn with_ease(mut self, kind: Ease) -> Tween {
        self.kind = kind;
        self
    }

    pub fn target(&self) -> f32 {
        self.to
    }

    pub fn value_at(&self, now: f64) -> f32 {
        if self.dur <= 0.0 || now >= self.start + self.dur {
            return self.to;
        }
        let t = ((now - self.start) / self.dur).max(0.0) as f32;
        lerp(self.from, self.to, ease(self.kind, t))
    }

    pub fn running_at(&self, now: f64) -> bool {
        self.dur > 0.0 && now < self.start + self.dur
    }

    /// 改變目標。與目前目標相同時不重新開始；dur 為 0 時直接跳到目標。
    pub fn retarget(&mut self, to: f32, now: f64, dur: f64) {
        if (to - self.to).abs() < 1e-4 {
            return;
        }
        if dur <= 0.0 {
            self.snap(to);
            return;
        }
        self.from = self.value_at(now);
        self.to = to;
        self.start = now;
        self.dur = dur;
    }

    /// 從指定值重新播放到目標（例如每次內容更新都淡入一次）。
    pub fn restart(&mut self, from: f32, to: f32, now: f64, dur: f64) {
        if dur <= 0.0 {
            self.snap(to);
            return;
        }
        self.from = from;
        self.to = to;
        self.start = now;
        self.dur = dur;
    }

    pub fn snap(&mut self, value: f32) {
        self.from = value;
        self.to = value;
        self.dur = 0.0;
    }
}

/// 清單／樹狀圖每一列的選取與懸停動畫。
///
/// 以「上次畫出來時的狀態」偵測變化：某列重畫時選取狀態和上次不同，才從舊狀態淡到新狀態；
/// 第一次出現（例如捲動進畫面）的列直接以目前狀態顯示，不播放動畫。
pub struct RowFx<K: Copy + Eq + Hash> {
    seen: HashMap<K, bool>,
    sel: HashMap<K, Tween>,
    hot: Option<K>,
    hot_fx: HashMap<K, Tween>,
}

impl<K: Copy + Eq + Hash> Default for RowFx<K> {
    fn default() -> Self {
        RowFx { seen: HashMap::new(), sel: HashMap::new(), hot: None, hot_fx: HashMap::new() }
    }
}

impl<K: Copy + Eq + Hash> RowFx<K> {
    /// 內容整個換掉（換資料夾、排序、篩選）時呼叫。
    pub fn clear(&mut self) {
        self.seen.clear();
        self.sel.clear();
        self.hot = None;
        self.hot_fx.clear();
    }

    /// 捲動後：忘記哪些列畫過，讓新進入畫面的列不播放動畫；進行中的動畫保留。
    pub fn reset_seen(&mut self) {
        self.seen.clear();
    }

    pub fn forget(&mut self, key: K) {
        self.seen.remove(&key);
        self.sel.remove(&key);
        self.hot_fx.remove(&key);
        if self.hot == Some(key) {
            self.hot = None;
        }
    }

    /// 畫某一列時呼叫，回傳（選取程度, 懸停程度），皆為 0～1。
    /// `sel_ms`、`hot_ms` 為 0 時不播放動畫（例如系統關閉了動畫效果）。
    pub fn observe(&mut self, key: K, selected: bool, hot: bool, now: f64, sel_ms: f64, hot_ms: f64) -> (f32, f32) {
        let target = if selected { 1.0 } else { 0.0 };
        match self.seen.insert(key, selected) {
            Some(prev) if prev != selected => {
                let tw = self.sel.entry(key).or_insert_with(|| Tween::new(if prev { 1.0 } else { 0.0 }));
                tw.retarget(target, now, sel_ms);
            }
            Some(_) => {
                if let Some(tw) = self.sel.get_mut(&key) {
                    tw.retarget(target, now, sel_ms);
                }
            }
            None => {
                self.sel.remove(&key);
            }
        }
        let s = match self.sel.get(&key) {
            Some(tw) => {
                let v = tw.value_at(now);
                if !tw.running_at(now) {
                    self.sel.remove(&key);
                }
                v
            }
            None => target,
        };

        if hot {
            if self.hot != Some(key) {
                if let Some(old) = self.hot.take() {
                    self.hot_fx.entry(old).or_insert_with(|| Tween::new(1.0)).retarget(0.0, now, hot_ms);
                }
                self.hot_fx.entry(key).or_insert_with(|| Tween::new(0.0)).retarget(1.0, now, hot_ms);
                self.hot = Some(key);
            }
        } else if self.hot == Some(key) {
            self.hot = None;
            self.hot_fx.entry(key).or_insert_with(|| Tween::new(1.0)).retarget(0.0, now, hot_ms);
        }
        let h = match self.hot_fx.get(&key) {
            Some(tw) => {
                let v = tw.value_at(now);
                if !tw.running_at(now) {
                    self.hot_fx.remove(&key);
                }
                v
            }
            None => {
                if self.hot == Some(key) {
                    1.0
                } else {
                    0.0
                }
            }
        };
        (s, h)
    }

    /// 仍在播放動畫的列（需要持續重畫）。
    pub fn animating(&self, now: f64) -> Vec<K> {
        let mut out: Vec<K> = self.sel.iter().filter(|(_, t)| t.running_at(now)).map(|(k, _)| *k).collect();
        for (k, t) in &self.hot_fx {
            if t.running_at(now) && !out.contains(k) {
                out.push(*k);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_endpoints() {
        for k in [Ease::Out, Ease::InOut, Ease::OutBack] {
            assert!((ease(k, 0.0)).abs() < 1e-5, "{k:?}");
            assert!((ease(k, 1.0) - 1.0).abs() < 1e-5, "{k:?}");
        }
        assert!(ease(Ease::Out, 0.5) > 0.5);
        assert!((ease(Ease::InOut, 0.5) - 0.5).abs() < 1e-5);
        // 回彈曲線途中會超過 1
        assert!((0..100).map(|i| ease(Ease::OutBack, i as f32 / 100.0)).any(|v| v > 1.0));
    }

    #[test]
    fn tween_runs_and_retargets_smoothly() {
        let mut t = Tween::new(0.0);
        t.retarget(1.0, 1000.0, 100.0);
        assert_eq!(t.value_at(1000.0), 0.0);
        assert!(t.running_at(1050.0));
        let mid = t.value_at(1050.0);
        assert!(mid > 0.5 && mid < 1.0);
        assert_eq!(t.value_at(1100.0), 1.0);
        assert!(!t.running_at(1100.0));
        // 途中改回 0：從目前的值接續
        let mut t = Tween::new(0.0);
        t.retarget(1.0, 0.0, 100.0);
        let v = t.value_at(50.0);
        t.retarget(0.0, 50.0, 100.0);
        assert!((t.value_at(50.0) - v).abs() < 1e-5);
        // 相同目標不重新開始
        let before = t;
        t.retarget(0.0, 60.0, 100.0);
        assert_eq!(t, before);
        // 不播放動畫時直接跳到目標
        t.retarget(1.0, 70.0, 0.0);
        assert_eq!(t.value_at(70.0), 1.0);
        assert!(!t.running_at(70.0));
    }

    #[test]
    fn row_fx_animates_changes_but_not_first_appearance() {
        let mut fx: RowFx<usize> = RowFx::default();
        // 第一次畫：直接是目前狀態
        assert_eq!(fx.observe(1, true, false, 0.0, 150.0, 120.0), (1.0, 0.0));
        assert_eq!(fx.observe(2, false, false, 0.0, 150.0, 120.0), (0.0, 0.0));
        assert!(fx.animating(0.0).is_empty());
        // 第 2 列被選取：從 0 開始淡入
        let (s, _) = fx.observe(2, true, false, 10.0, 150.0, 120.0);
        assert_eq!(s, 0.0);
        assert_eq!(fx.animating(20.0), vec![2]);
        let (s, _) = fx.observe(2, true, false, 85.0, 150.0, 120.0);
        assert!(s > 0.0 && s < 1.0);
        let (s, _) = fx.observe(2, true, false, 200.0, 150.0, 120.0);
        assert_eq!(s, 1.0);
        assert!(fx.animating(200.0).is_empty());
        // 捲動後重新出現：不播放
        fx.reset_seen();
        assert_eq!(fx.observe(2, false, false, 300.0, 150.0, 120.0), (0.0, 0.0));
    }

    #[test]
    fn row_fx_hover_moves_between_rows() {
        let mut fx: RowFx<usize> = RowFx::default();
        fx.observe(1, false, false, 0.0, 150.0, 100.0);
        let (_, h) = fx.observe(1, false, true, 0.0, 150.0, 100.0);
        assert_eq!(h, 0.0);
        let (_, h) = fx.observe(1, false, true, 200.0, 150.0, 100.0);
        assert_eq!(h, 1.0);
        // 移到第 3 列：第 3 列淡入，第 1 列淡出
        fx.observe(3, false, true, 300.0, 150.0, 100.0);
        let mut busy = fx.animating(310.0);
        busy.sort();
        assert_eq!(busy, vec![1, 3]);
        let (_, h1) = fx.observe(1, false, false, 350.0, 150.0, 100.0);
        assert!(h1 > 0.0 && h1 < 1.0);
        let (_, h1) = fx.observe(1, false, false, 450.0, 150.0, 100.0);
        assert_eq!(h1, 0.0);
        // 動畫關閉時直接切換
        let mut fx: RowFx<usize> = RowFx::default();
        fx.observe(5, false, false, 0.0, 0.0, 0.0);
        assert_eq!(fx.observe(5, true, true, 1.0, 0.0, 0.0), (1.0, 1.0));
        assert!(fx.animating(1.0).is_empty());
    }
}
