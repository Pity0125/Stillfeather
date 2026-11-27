# Stillfeather v0.1.7

## 新增功能
- 側欄可以自訂標籤：在側欄的空白處或標籤的標題按右鍵選「新增標籤…」，就多一個和「常用位置」「磁碟」同一層的標籤（例如「工作」「照片」），把資料夾拖到標籤的標題上就放進去。0.1.6 在「常用位置」底下建立的書籤分類會直接變成自訂標籤。標籤可以重新命名、刪除（裡面的資料夾移回「常用位置」），按住標題上下拖曳可以調整順序，點標題可以收起或展開。
- 中鍵點資料夾在新分頁開啟：清單、側欄與路徑列的資料夾都可以。和瀏覽器一樣插在目前分頁的後面、不切過去，可以一口氣開好幾個；按住 Shift 時切過去。
- 路徑列可以執行程式，效果同 Win+R：程式、批次檔、捷徑直接執行；`cmd`、`notepad` 這類指令與網址交給 Windows 執行，工作資料夾是目前的資料夾，所以輸入 `cmd` 就在這裡開命令提示字元。`shell:startup`、`shell:downloads` 這類位置也能開。
- 最近搜尋：在搜尋框按 ↓ 列出最近 10 次搜尋，連篩選條件與範圍一起記，選一個就照樣再搜一次；最下面可以清除。

## 改進
- 「設定 › 快速鍵」改版：依功能分成五頁，點一下命令再直接按想要的組合鍵就設定好，Esc 取消。已經是別的命令的按法時會先問要不要改過來，不能用的按法會在那一列說明原因；改過的命令前面有小圓點，「還原」改回預設，「不用快速鍵」停用。
- 搜尋框空著時，提示會寫出這次會搜哪裡：「在「下載」及子資料夾中搜尋…」「只在「下載」中搜尋…」「搜尋整台電腦…」。

## 修正
- 用滑鼠拖曳框選不再卡卡的，方框拉得很大時也跟得上滑鼠，選取的底色和方框同步出現。圖示、並排等檢視框選時，狀態列也不再一直更新。

## 下載
- StillfeatherSetup.exe：安裝程式（已安裝的 Stillfeather 會自動提醒更新）
- RestoreExplorer.cmd：Stillfeather 打不開時，把資料夾、Win+E 與開啟檔案視窗改回 Windows 的
- update.json、update.json.sig：自動更新用，不需要下載

---

## New features
- Your own sidebar sections: right-click empty space in the sidebar or a section heading and choose New section… to add a section alongside Favorites and Drives (for example Work or Photos), then drag folders onto its heading to add them. Bookmark groups created under Favorites in 0.1.6 become sections automatically. Sections can be renamed or deleted (their folders go back to Favorites), dragged up or down by the heading to reorder them, and collapsed or expanded by clicking the heading.
- Middle-click a folder to open it in a new tab, in the list, the sidebar or the path bar. Like a browser, the tab opens next to the current one without switching to it, so you can open several in a row; hold Shift to switch to it.
- The path bar runs things like Win+R: programs, batch files and shortcuts run directly; commands such as `cmd` and `notepad`, and web addresses, are handed to Windows and start in the current folder, so typing `cmd` opens a command prompt right here. Locations such as `shell:startup` and `shell:downloads` open too.
- Recent searches: press ↓ in the search box to list your last 10 searches, remembered with their filters and scope; pick one to run it again, or clear the list from the bottom.

## Improvements
- Settings › Shortcuts is redesigned and split into five pages by function. Click a command and press the key combination you want, or Esc to cancel. If another command already uses it, you're asked before it moves; combinations that can't be used explain why on that row. Changed commands are marked with a dot, Reset returns a command to its default, and No shortcut turns it off.
- While the search box is empty, its hint tells you where it will search: “Search “Downloads” and its subfolders…”, “Search only in “Downloads”…” or “Search the whole PC…”.

## Fixes
- Box selection with the mouse no longer lags, even with a very large box, and the selection highlight appears together with the box. In Icons, Tiles and similar views, the status line no longer updates constantly while you drag.

## Downloads
- StillfeatherSetup.exe: the installer (installed copies of Stillfeather offer the update automatically)
- RestoreExplorer.cmd: switches folders, Win+E and file dialogs back to Windows if Stillfeather won't open
- update.json, update.json.sig: used by automatic updates, no need to download
