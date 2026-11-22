//! 依位元組預算淘汰的小型「最近使用」快取。
//!
//! 快速預覽用：滑鼠在幾個項目之間來回時，已經產生過的縮圖與上色好的文字直接重用，
//! 不必每換一次就再啟動縮圖工作程序。項目只有幾十個，線性搜尋就夠快。

pub struct ByteLru<K, V> {
    /// 舊的在前、最近用過的在後：（鍵, 值, 估計的位元組數）
    items: Vec<(K, V, usize)>,
    bytes: usize,
    max_bytes: usize,
    max_items: usize,
}

impl<K, V> ByteLru<K, V> {
    /// 總大小不超過 `max_bytes`、項目不超過 `max_items` 個。可用於 `static`。
    pub const fn new(max_bytes: usize, max_items: usize) -> Self {
        ByteLru { items: Vec::new(), bytes: 0, max_bytes, max_items }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 目前保存的估計位元組數
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.bytes = 0;
    }
}

impl<K: PartialEq, V: Clone> ByteLru<K, V> {
    /// 取出一份複本，並標示為最近用過。
    pub fn get(&mut self, key: &K) -> Option<V> {
        let i = self.items.iter().position(|(k, _, _)| k == key)?;
        let item = self.items.remove(i);
        let value = item.1.clone();
        self.items.push(item);
        Some(value)
    }

    /// 放入（同一個鍵會取代舊的）；空間不夠時先丟掉最久沒用的。單一項目超過整個預算就不保存。
    pub fn put(&mut self, key: K, value: V, bytes: usize) {
        if let Some(i) = self.items.iter().position(|(k, _, _)| *k == key) {
            let (_, _, old) = self.items.remove(i);
            self.bytes -= old;
        }
        if bytes > self.max_bytes || self.max_items == 0 {
            return;
        }
        while !self.items.is_empty() && (self.bytes + bytes > self.max_bytes || self.items.len() >= self.max_items) {
            let (_, _, old) = self.items.remove(0);
            self.bytes -= old;
        }
        self.items.push((key, value, bytes));
        self.bytes += bytes;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_and_miss() {
        let mut c: ByteLru<&str, i32> = ByteLru::new(100, 10);
        assert_eq!(c.get(&"a"), None);
        c.put("a", 1, 10);
        assert_eq!(c.get(&"a"), Some(1));
        assert_eq!(c.get(&"b"), None);
        assert_eq!((c.len(), c.bytes()), (1, 10));
    }

    #[test]
    fn evicts_least_recently_used() {
        let mut c: ByteLru<&str, i32> = ByteLru::new(1000, 3);
        c.put("a", 1, 1);
        c.put("b", 2, 1);
        c.put("c", 3, 1);
        // 用過 a：最久沒用的變成 b
        assert_eq!(c.get(&"a"), Some(1));
        c.put("d", 4, 1);
        assert_eq!(c.get(&"b"), None);
        assert_eq!(c.get(&"a"), Some(1));
        assert_eq!(c.get(&"c"), Some(3));
        assert_eq!(c.get(&"d"), Some(4));
        assert_eq!(c.len(), 3);
    }

    #[test]
    fn respects_byte_budget() {
        let mut c: ByteLru<&str, i32> = ByteLru::new(100, 10);
        c.put("a", 1, 40);
        c.put("b", 2, 40);
        c.put("c", 3, 40);
        assert_eq!(c.get(&"a"), None);
        assert_eq!(c.bytes(), 80);
        // 單一項目超過預算：不保存，也不把別的擠掉
        c.put("huge", 9, 101);
        assert_eq!(c.get(&"huge"), None);
        assert_eq!(c.len(), 2);
        // 同一個鍵取代舊值，大小重新計算
        c.put("b", 20, 10);
        assert_eq!(c.get(&"b"), Some(20));
        assert_eq!(c.bytes(), 50);
        c.clear();
        assert!(c.is_empty());
        assert_eq!(c.bytes(), 0);
    }
}
