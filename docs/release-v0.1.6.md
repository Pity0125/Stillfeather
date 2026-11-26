# v0.1.6｜分頁獨立視窗、書籤分類、自訂快速鍵、進階搜尋與八組主題 / Detachable Tabs, Bookmark Groups, Custom Shortcuts, Advanced Search & Eight Themes

## 繁體中文

### 新增功能

- **分頁獨立視窗**：把分頁拖出視窗邊界即可獨立開啟；視窗最大化時也可以往分頁列下方拖。新視窗載入資料夾後才關閉原分頁。單欄只剩一個分頁時會直接移動視窗。重新啟動時只還原主視窗的分頁。
- **書籤分類**：在「常用位置」按右鍵新增分類，可拖入資料夾、移動既有書籤或重新命名分類。移除分類時保留書籤。
- **自訂快速鍵**：在「設定 › 快速鍵」選擇命令，輸入一組或多組按法，留白可停用。遇到衝突時會提示，儲存後立即套用。

### 改進
- 新增八組調好的主題：陶土（原本的配色）、鼠尾草、海港、薰衣草、玫瑰、琥珀、湖水綠、石墨，每一組都有淺色與深色。在設定新增的「主題與顏色」分類選，選項上直接畫出那個主題的樣子。背景只帶一點點色調，顏色主要在重點色上，換了主題看得出來，又不會整片染色。自訂重點色與背景色移到同一頁的「自訂顏色（進階）」，之前設定過的人照樣沿用；兩個顏色後面各有「重設」按鈕，一按就改回主題的顏色。
- 設定的「外觀與文字」改名為「文字與語言」，只放文字大小與介面語言；深淺色移到「主題與顏色」。
- 拿掉側欄底部的「個人化」卡片（和右上角的齒輪重複）與頂列的深淺色切換按鈕：深淺色在設定 › 主題與顏色切換，側欄的資料夾樹一路到底。
- 詳細資料檢視的「所在位置」欄窄一點，右邊留下約清單寬度 1/6 的空白，可以點空白處取消選取、從那裡框選；位置被截斷時，滑鼠停在那一格就在下方顯示完整位置。
- 設定的「預覽」簡化成一個開關：先勾選「啟用快速預覽」，下面的「預覽方式」下拉選單選「懸停預覽」或「空白鍵預覽」，說明跟著選的方式換。關閉時滑鼠停留、空白鍵都不會預覽，右鍵選單的「快速預覽」照樣可以用。原本「快速預覽圖片與影片」「快速預覽 PDF」的個別開關拿掉了，開啟快速預覽時一律預覽這些檔案。
- Shift+Delete 永久刪除（按住 Shift 點「刪除」也一樣，和檔案總管相同）：一定先確認，預設焦點在「取消」，不會一路按 Enter 就刪掉；永久刪除不能復原。
- 切換雙欄改成 Ctrl+\（同 VS Code 的分割編輯器），不再和 Ctrl+D 刪除擠在同一個鍵上；舊的 Ctrl+Shift+D 暫時保留，按了只會提示新的按鍵。
- 分頁的快速鍵和瀏覽器相同：Ctrl+1～8 跳到第 N 個分頁、Ctrl+9 跳到最後一個、Ctrl+PgUp／PgDn 切換分頁、Ctrl+Shift+T 重新開啟剛關掉的分頁（放回原來的位置，上一頁紀錄也還在）。
- 分頁可以拖曳調整順序：按住分頁拖到想要的位置，會有一條重點色的線標出放下的地方；雙欄時也能拖到另一欄的分頁列。按 Esc 取消。跨欄移動時，來源欄至少會保留一個分頁。
- 分頁改成 Chrome 的樣子：選取的分頁是和下面清單同色的分頁形狀（上緣圓角、底下兩側往外彎），和清單連成一片，一眼看得出是哪一個；其他分頁沒有底色，滑鼠停著時有淡淡的圓角底色。每個分頁前面有資料夾圖示（搜尋結果是放大鏡），夠寬時每個分頁都有關閉按鈕。原本的重點色底線拿掉了，雙欄時作用中那一欄的分頁和清單一起用重點色描邊。
- 搜尋框展開後更寬（最寬約 560 像素、跟著文字大小放大，最多佔路徑列那一列的一半多一點），輸入比較長的檔名時看得到全部。
- 輸入路徑的快速鍵改成 Ctrl+E（Alt+D 照舊）；按舊的 Ctrl+L 時狀態列會提示新的按鍵。滑鼠停在路徑列的空白處時，右邊也會提示「點一下或按 Ctrl+E 輸入路徑」。
- 網路位置：路徑列輸入 `\\nas` 就列出那台 NAS 或電腦分享的資料夾；麵包屑把電腦和共用資料夾分成兩段，點電腦那一段回到清單。需要帳號密碼時直接跳出 Windows 的登入視窗（可以記住認證），登入後自動開啟，不必先到檔案總管連一次。找不到電腦、已經用另一組帳號連線、Windows 封鎖訪客存取時，錯誤訊息會說明原因。
- 搜尋多了篩選條件：類型篩選籤後面有「日期 ▾」（今天、昨天、本週、本月、今年、自訂範圍）與「大小 ▾」（小於 1 MB、1～100 MB、100 MB～1 GB、大於 1 GB、自訂）兩個按鈕，選了的條件顯示在按鈕上（「本週 ×」），按 × 清除。條件一改，現有結果直接跟著篩選，不必重新搜尋；結果太多被截斷時才帶著條件重新搜尋，並在狀態列說明。只選條件、搜尋框空著也能搜尋（在「下載」選「文件」「本週」按 Enter，就列出這週的文件）。狀態列顯示「找到 86 個 · 篩選：文件、本週、> 10 MB · 清除篩選」，找不到時一樣可以直接按「清除篩選」。資料夾清單也能用這兩個條件篩選。
- 搜尋框可以寫條件：多個詞都要有（順序不限）、`"年度 報告"` 完整詞組、`-草稿` 排除、`*.pdf` 萬用字元、`ext:pdf;docx`、`size:>100mb`、`size:1mb..10mb`、`dm:today`、`dm:7d`、`dm:2026-11-01..2026-11-20`（也可以寫 `datemodified:`）、`folder:`、`file:`；冒號用全形的「：」也可以。條件在搜尋時就先判斷，不符合的不會佔掉 50,000 個結果的上限。說明頁的「搜尋」有完整的語法表。
- 搜尋結果預設依相關性排序：檔名完全相同的最前面，再來是開頭相同、在詞的開頭出現、中間有出現；同一級時路徑淺的、比較新的在前。結果邊找邊照順序插進去，選取和捲動位置不會跳。搜尋中也可以按欄位標題換排序，「排序 › 相關性」換回來；回到資料夾時恢復原本的排序。
- 刪除到資源回收筒、移動（剪下貼上、拖放）不再每次確認；丟資源回收筒要不要確認改成照 Windows 的設定（資源回收筒「內容」的「顯示刪除確認對話方塊」）。完成後狀態列用重點色提示「Ctrl+Z 可復原」。
- 說明改成獨立的說明頁（F1 或右上角的 ?）：原本的說明塞在一個對話方塊裡，快速鍵加上說明放不下。新的說明頁是可以縮放、捲動的視窗，不擋住主視窗，可以一邊看一邊操作；左邊分成快速鍵、搜尋、快速預覽、檔案與資料夾、檢視與版面、關於。快速鍵一覽從快速鍵表自動產生，依分頁與雙欄、檢視、移動與搜尋、檔案與資料夾分段，快速預覽的按鍵依目前的設定；搜尋語法、篩選條件與相關性排序都在「搜尋」那一頁，正在搜尋時按 F1 直接開在這一頁。Ctrl+C 複製整頁文字，Esc 關閉。

### 內部
- 主視窗的程式碼依功能拆開：原本四千多行的 `app.rs` 只留主視窗本體（建立、布局、繪製、狀態同步、命令），分頁的狀態、導覽、搜尋、分頁與雙欄、檔案操作、設定變更、鍵盤與視窗程序各自放在 `src/win/app/` 底下的檔案。行為不變。
- 快速鍵表可以在執行時套上自訂的按法（`Keymap`）：命令有固定的英文代號、按法可以從「Ctrl+Shift+T」這樣的文字解析，衝突一次列出來。主視窗、選擇器與說明一覽都改查這份表。自訂綁定儲存在設定檔的 `Shortcuts` 欄位。

### 修正
- 文字大小調大後，「排序」「檢視」、篩選籤的「日期」「大小」、分頁的右鍵選單與設定裡「預覽方式」這些下拉選單的字也一起放大，列高與寬度跟著變；選單的顏色也跟著 Stillfeather 的主題。以前這些選單是 Windows 畫的，字型固定是系統的大小。檔案的右鍵選單混有 Windows 自己的項目（傳送到、內容等），仍維持系統的大小。
- 自訂背景色時配色不再怪異：以前會把主題的側欄色混進自訂背景，淺色模式配深色背景時側欄變成一片灰、和清單深淺不一致；現在卡片、側欄與文字都只從自訂的背景推導，深淺也跟著背景。
- 文字大小調大後，路徑列、子資料夾選單、篩選籤、搜尋框，以及刪除確認、錯誤訊息、使用說明這些對話方塊的字也一起放大，列高跟著加高，不再只有清單變大。
- 深色模式的標題列在 Windows 10 上也會變深色（1809～1909 用的是舊的屬性編號），切換深淺色時馬上換色，不必等視窗重新啟用；開著的對話方塊也會一起換。
- 新增資料夾後一定會馬上進入改名（以前有時只建立了「新增資料夾」，沒有進入編輯）：新資料夾直接加進清單，不再重新讀取整個資料夾；篩選「圖片」等類型時會換回「全部」，才看得到新資料夾。Windows 右鍵選單的「新增 ▸」（資料夾、文字文件等）建立的項目也會選取並進入改名，和檔案總管相同。Ctrl+Shift+N 也能新增資料夾。
- 在一個分頁刪除或移動資料夾後，切到正在顯示那個資料夾（或它底下）的另一個分頁，不再留著已刪除資料夾的舊內容：刪除時改顯示還在的上一層，移動時跟到新位置，並在狀態列說明。被其他程式（例如檔案總管）刪掉的資料夾，切回分頁時也一樣改顯示上一層。

## English

### Tabs, bookmarks, and shortcuts

- **Detach tabs.** Drop a tab outside the window. When maximized, drag it down into the file list. The source tab stays open until the destination loads its folder. Dragging the only tab in a single-pane window moves that window. Session restore remembers tabs in the main window.
- **Bookmark groups.** Right-click Favorites to create a group, then drop folders onto it to pin them. Move existing bookmarks from their context menu. Rename a group or remove it and keep its bookmarks.
- **Custom shortcuts.** In Settings › Shortcuts, select a command and enter one or more key combinations separated by a semicolon. Leave the field empty to disable a binding. A conflict prompt identifies affected commands. Saving applies the changes immediately.
- Drag tabs to reorder them or move them between panes. An insertion line marks the destination and Esc cancels. Cross-pane moves leave at least one tab in the source pane. The selected tab blends into the list, folder and search tabs have distinct icons, wider tabs show close buttons, and an accent outline identifies the active pane.
- Ctrl+1–8 selects a numbered tab, Ctrl+9 selects the last, and Ctrl+PgUp/PgDn switches tabs. Ctrl+Shift+T reopens a closed tab in its original position with its navigation history. Ctrl+\ toggles dual panes. Ctrl+E and Alt+D edit the path. The old Ctrl+Shift+D and Ctrl+L bindings display reminders. Hover over empty space in the path bar for the edit hint.

### Search and network shares

- The expanded search box is up to about 560 pixels wide at standard text size and scales with the window and font. Date and Size filters apply to folders and search results. Pick today, yesterday, this week, month, year, or a custom date range. Size presets cover under 1 MB, 1–100 MB, 100 MB–1 GB, and over 1 GB, with custom ranges available.
- Search without entering a name by choosing filters, such as Documents and This week in Downloads. Click × to remove a condition or clear all filters from the status line, even with zero results. Results update immediately. If earlier results were truncated, Stillfeather searches again with the new filters and explains why.
- Combine words in any order, exact phrases (`"annual report"`), exclusions (`-draft`), wildcards (`*.pdf`), extensions (`ext:pdf;docx`), sizes (`size:>100mb`, `size:1mb..10mb`), dates (`dm:today`, `dm:7d`, `dm:2026-11-01..2026-11-20`), and `file:` or `folder:`. The parser accepts `datemodified:` and full-width colons. Filters apply before the 50,000-result limit.
- Relevance ranks exact names first, followed by name prefixes, word prefixes, and other matches. Shallower paths and newer items break ties. Incoming results preserve selection and scroll position. Click a column to change sorting, choose Sort › Relevance to switch back, or leave search to restore folder sorting.
- Enter `\\nas` to list a computer's shares. Separate computer and share breadcrumbs let you return to the share list. Windows prompts for credentials, can remember them, and continues opening the folder after sign-in. Errors distinguish an unreachable computer, conflicting credentials, and blocked guest access.

### Appearance, preview, and Help

- Settings › Theme & colors previews eight palettes (Clay, Sage, Harbor, Lavender, Rose, Amber, Teal, and Graphite), each with light and dark variants. Existing accent and background overrides carry over and can be reset individually. Cards, the sidebar, and text adapt to custom backgrounds.
- Text & language contains text size and interface language. Light and dark mode are under Theme & colors. Removing the sidebar personalization card and top-bar theme toggle gives the folder tree the full sidebar height.
- In Details view, a narrower Location column leaves about one sixth of the width free. Click there to clear a selection or drag to box-select. Hover over a clipped location to read its full path.
- One Quick preview switch now covers images, videos, and PDFs. Choose hover or Space to trigger it. Context-menu preview remains available when the switch is off.
- Larger text scales the path bar, search box, filter chips, subfolder menus, dialogs, and Help. Sort, View, Date, Size, tab, and preview-mode menus resize and follow the theme. Shell context menus retain the Windows text size. Dark title bars now update immediately on Windows 10, including versions 1809–1909. Open dialogs also follow theme changes.
- F1 or the ? button opens a resizable, scrollable Help window alongside the main window. It covers shortcuts, search, preview, files and folders, views and layout, and app information. The shortcut list reflects current bindings and preview settings. F1 during search opens the search syntax guide. Ctrl+C copies the page and Esc closes Help.

### File operations and fixes

- Recycle Bin confirmation follows Windows preferences. Moves by cut-and-paste or drag-and-drop skip the extra confirmation. The status line highlights Ctrl+Z when undo is available. Shift+Delete or Shift-clicking Delete permanently deletes items after confirmation, with Cancel focused by default. Permanent deletion cannot be undone.
- Newly created folders appear immediately in rename mode, including with Ctrl+Shift+N. A filter that would hide the folder switches to All. Items created from the Windows New submenu are also selected for renaming.
- Tabs follow a folder or parent folder moved in another tab. When the folder disappears, switching back opens a surviving parent and explains the change. This also handles folders deleted by another application.

### Internal changes

- Tab state, navigation, search, tab management, file actions, settings, keyboard handling, and the window procedure now live in separate modules under `src/win/app/`. The main module handles window creation, layout, state synchronization, and commands.
- The main window, file picker, and Help share one runtime keymap. Stable command IDs and parsed key combinations support conflict reporting. Custom bindings are saved in the `Shortcuts` settings field.
