# Stillfeather v0.1.6

## 新增功能
- 分頁可以獨立成新視窗：把分頁拖出視窗邊界就會在新視窗開啟；視窗最大化時，往分頁列下方拖也可以。新視窗載入資料夾後才關閉原分頁；單欄只剩一個分頁時直接移動整個視窗。重新啟動時只還原主視窗的分頁。
- 「常用位置」可以分類：在「常用位置」按右鍵新增書籤分類，把資料夾拖進去就能釘選；既有的書籤可以移到別的分類，分類也能重新命名。移除分類時，裡面的書籤會保留。
- 快速鍵可以自訂：在「設定 › 快速鍵」選擇命令，輸入一組或多組按法（多組用 `;` 分隔），留白就停用。和其他命令衝突時會列出受影響的命令，儲存後立即套用。

## 改進
- 八組主題：陶土（原本的配色）、鼠尾草、海港、薰衣草、玫瑰、琥珀、湖水綠、石墨，每組都有淺色與深色，在設定新增的「主題與顏色」選，選項上直接畫出主題的樣子。自訂重點色與背景色移到同一頁的「自訂顏色（進階）」，之前的設定照樣沿用，各有「重設」按鈕可以改回主題的顏色。
- 設定的「外觀與文字」改名為「文字與語言」，只放文字大小與介面語言；深淺色移到「主題與顏色」。側欄底部的「個人化」卡片和頂列的深淺色按鈕拿掉了，側欄的資料夾樹一路到底。
- 分頁和瀏覽器一樣：Ctrl+1～8 跳到第 N 個分頁、Ctrl+9 跳到最後一個、Ctrl+PgUp／PgDn 切換，Ctrl+Shift+T 重新開啟剛關掉的分頁（放回原位，上一頁紀錄也在）。分頁可以拖曳排序，雙欄時也能拖到另一欄，按 Esc 取消。
- 分頁改成 Chrome 的樣子：選取的分頁和下面的清單連成一片，每個分頁前面有資料夾圖示（搜尋結果是放大鏡），夠寬時都有關閉按鈕；雙欄時作用中那一欄用重點色描邊。
- 快速鍵調整：切換雙欄改成 Ctrl+\，不再和 Ctrl+D 刪除擠在一起；輸入路徑改成 Ctrl+E（Alt+D 照舊）。按舊的 Ctrl+Shift+D、Ctrl+L 只會提示新的按鍵。
- 搜尋多了「日期 ▾」與「大小 ▾」篩選，條件一改結果就跟著篩選；只選條件、不輸入檔名也能搜尋（例如在「下載」選「文件」「本週」按 Enter）。資料夾清單也能用這兩個條件篩選。
- 搜尋框可以寫條件：多個詞、`"年度 報告"` 完整詞組、`-草稿` 排除、`*.pdf`、`ext:pdf;docx`、`size:>100mb`、`dm:7d`、`folder:`、`file:` 等，完整語法在說明頁的「搜尋」。
- 搜尋結果預設依相關性排序：檔名完全相同的最前面，再來是開頭相同、在詞的開頭出現、中間有出現；結果邊找邊插進去，選取和捲動位置不會跳。也可以按欄位標題換排序，「排序 › 相關性」換回來。
- 搜尋框展開後更寬，輸入長檔名時看得到全部。
- 網路位置：路徑列輸入 `\\nas` 就列出那台 NAS 或電腦分享的資料夾。需要帳號密碼時直接跳出 Windows 的登入視窗，登入後自動開啟；找不到電腦、已經用另一組帳號連線、Windows 封鎖訪客存取時，錯誤訊息會說明原因。
- 刪除到資源回收筒、剪下貼上與拖放移動不再每次確認；資源回收筒要不要確認照 Windows 的設定，完成後狀態列提示「Ctrl+Z 可復原」。Shift+Delete（或按住 Shift 點「刪除」）永久刪除，一定先確認，預設焦點在「取消」。
- 快速預覽簡化成一個開關：勾選「啟用快速預覽」，再選「懸停預覽」或「空白鍵預覽」。關閉時，右鍵選單的「快速預覽」照樣可以用。
- 詳細資料檢視的「所在位置」欄窄一點，右邊留下空白，方便取消選取與框選；位置被截斷時，滑鼠停著就顯示完整位置。
- 說明改成獨立的說明頁（F1 或右上角的 ?）：可以縮放、捲動，不擋住主視窗；快速鍵一覽依目前的設定自動產生，搜尋時按 F1 直接開在「搜尋」那一頁。

## 修正
- 新增資料夾後一定馬上進入改名（以前有時只建立、沒有進入編輯）；篩選「圖片」等類型時會換回「全部」。Ctrl+Shift+N 和 Windows 右鍵選單的「新增 ▸」也一樣。
- 在一個分頁刪除或移動資料夾後，另一個開著那個資料夾的分頁不再留著舊內容：刪除時改顯示上一層，移動時跟到新位置，並在狀態列說明。被其他程式刪掉時也一樣。
- 文字大小調大後，路徑列、子資料夾選單、篩選籤、搜尋框、對話方塊與下拉選單也一起放大，選單的顏色也跟著主題。檔案的右鍵選單含有 Windows 自己的項目，仍維持系統的大小。
- 自訂背景色時配色不再怪異：以前淺色模式配深色背景會讓側欄變成一片灰，現在卡片、側欄與文字都跟著自訂的背景。
- Windows 10 的深色模式標題列也會變深色（包括 1809～1909），切換深淺色時馬上換色，開著的對話方塊也會一起換。

## 下載
- StillfeatherSetup.exe：安裝程式（已安裝的 Stillfeather 會自動提醒更新）
- RestoreExplorer.cmd：Stillfeather 打不開時，把資料夾、Win+E 與開啟檔案視窗改回 Windows 的
- update.json、update.json.sig：自動更新用，不需要下載

---

## New features
- Detachable tabs: drag a tab past the window's edge to open it in a new window; when the window is maximized, dragging it below the tab bar works too. The original tab closes only after the new window has loaded the folder, and dragging the only tab in a single pane moves the whole window. Only the main window's tabs are restored on the next start.
- Bookmark groups in Favorites: right-click Favorites to create a group, then drag folders into it to pin them. Existing bookmarks can move to another group, and groups can be renamed. Removing a group keeps its bookmarks.
- Custom shortcuts: in Settings › Shortcuts, pick a command and enter one or more key combinations (separate them with `;`), or leave it blank to disable it. Conflicts list the affected commands, and changes apply as soon as you save.

## Improvements
- Eight themes: Clay (the original colors), Sage, Harbor, Lavender, Rose, Amber, Teal and Graphite, each with light and dark variants, under the new Settings › Theme & colors, where each option shows what the theme looks like. Custom accent and background colors move to Custom colors (advanced) on the same page; existing choices carry over, and each has a Reset button that returns it to the theme's color.
- Appearance & text in Settings is now Text & language, with just text size and interface language; light and dark mode move to Theme & colors. The sidebar's personalization card and the top bar's light/dark button are gone, so the folder tree runs the full height of the sidebar.
- Tabs work like a browser's: Ctrl+1–8 jumps to a tab, Ctrl+9 to the last one, Ctrl+PgUp/PgDn switches tabs, and Ctrl+Shift+T reopens the last closed tab in its old spot with its history. Drag tabs to reorder them, or to the other pane in dual-pane mode; Esc cancels.
- Tabs look like Chrome's: the selected tab blends into the list below it, each tab has a folder icon (a magnifier for search results), wide tabs all have close buttons, and in dual-pane mode the active pane is outlined in the accent color.
- Shortcut changes: toggling dual panes is now Ctrl+\, no longer sharing a key with Ctrl+D (delete); editing the path is now Ctrl+E (Alt+D still works). The old Ctrl+Shift+D and Ctrl+L just remind you of the new keys.
- Search gets Date ▾ and Size ▾ filters, and results update as soon as you change them. You can search with filters alone, without typing a name (for example, pick Documents and This week in Downloads, then press Enter). Folder lists can use both filters too.
- Type conditions in the search box: several words, exact phrases (`"annual report"`), exclusions (`-draft`), `*.pdf`, `ext:pdf;docx`, `size:>100mb`, `dm:7d`, `folder:`, `file:` and more. The full syntax is on the Search page of Help.
- Search results are ranked by relevance by default: exact name matches first, then names that start with the term, words that start with it, and other matches. Results slot in as they're found without moving your selection or scroll position. Click a column header to sort differently, and choose Sort › Relevance to switch back.
- The expanded search box is wider, so long file names fit.
- Network locations: type `\\nas` in the path bar to list the folders that NAS or computer shares. If it needs a user name and password, Windows' sign-in window appears and the folder opens once you sign in. Errors explain an unreachable computer, a connection that already uses other credentials, or guest access blocked by Windows.
- Moving to the Recycle Bin, cut-and-paste and drag-and-drop moves no longer ask every time; whether the Recycle Bin asks follows your Windows setting, and the status line reminds you that Ctrl+Z undoes it. Shift+Delete (or Shift-clicking Delete) deletes permanently, always after a confirmation with Cancel focused.
- Quick preview is one switch: check Quick preview, then choose Preview on hover or Preview with Space. When it's off, Quick preview in the right-click menu still works.
- In Details view, the Location column is narrower, leaving empty space on the right for clearing the selection and box-selecting; hover over a clipped location to see all of it.
- Help is now a separate window (F1 or the ? at the top right) that you can resize and scroll while you keep working. The shortcut list follows your current settings, and pressing F1 while searching opens the Search page.

## Fixes
- New folders always go straight into rename mode (before, the folder was sometimes created without it); a filter such as Images switches back to All so you can see it. Ctrl+Shift+N and Windows' New ▸ menu work the same way.
- After deleting or moving a folder in one tab, another tab showing that folder no longer keeps its old contents: it shows the parent folder after a delete, follows the folder after a move, and explains it in the status line. Folders deleted by other programs are handled too.
- With larger text, the path bar, subfolder menus, filter chips, search box, dialogs and drop-down menus all scale up, and the menus follow the theme's colors. File context menus, which include Windows' own items, keep the system size.
- Custom background colors no longer look odd: before, a dark background in light mode turned the sidebar a flat gray; now cards, the sidebar and text all follow the custom background.
- On Windows 10 (including 1809–1909), dark mode now darkens the title bar too, and switching between light and dark changes it right away, including in open dialogs.

## Downloads
- StillfeatherSetup.exe: the installer (installed copies of Stillfeather offer the update automatically)
- RestoreExplorer.cmd: switches folders, Win+E and file dialogs back to Windows if Stillfeather won't open
- update.json, update.json.sig: used by automatic updates, no need to download
