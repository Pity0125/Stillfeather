# v0.1.6｜分頁獨立視窗、書籤分類、自訂快速鍵、進階搜尋與八組主題 / Detachable Tabs, Bookmark Groups, Custom Shortcuts, Advanced Search & Eight Themes

## 新增功能

- **分頁可以拖出成獨立視窗。** 把分頁拖出視窗邊界即可開啟新視窗；視窗最大化時，往分頁列下方拖也可以。新視窗載入資料夾後才會關閉原分頁。單欄只剩一個分頁時會直接移動視窗。下次啟動時還原的是主視窗的分頁。
- **常用位置可以新增書籤分類。** 在側欄「常用位置」按右鍵建立分類，把資料夾拖進去即可釘選。已釘選的書籤可以移動分類，分類也能重新命名或移除。移除分類後，裡面的書籤會保留在常用位置。
- **快速鍵可以自己改。** 在「設定 › 快速鍵」選擇命令，輸入一組或多組按法，多組之間用半形分號分隔，留白可停用。遇到衝突時會列出受影響的命令，儲存後立即生效。

## 修正

- 新增資料夾後會立即選取並進入改名。使用 `Ctrl+Shift+N` 或 Windows 右鍵選單的「新增」也適用；若目前的篩選會隱藏新資料夾，清單會切回「全部」。
- 在另一個分頁移動使用中的資料夾或其上層資料夾後，切回原分頁會跟到新位置。資料夾被刪除時，會退回仍存在的上層並在狀態列說明。從其他程式刪除也適用。
- 放大文字後，路徑列、子資料夾選單、搜尋框、篩選籤、對話方塊及說明頁會一起放大。「排序」「檢視」「日期」「大小」、分頁右鍵選單與預覽方式選單也會調整列高、寬度與配色。含 Windows 擴充項目的檔案右鍵選單沿用系統字級。
- 自訂背景色後，側欄、卡片與文字會依背景深淺配色。Windows 10 1809～1909 的深色標題列也能正確顯示，切換主題時主視窗與開著的對話方塊會立即換色。

## 改進

- **分頁操作更接近瀏覽器。** 拖曳可調整順序或移至另一欄，插入線會標示放置位置，按 `Esc` 取消。跨欄移動時來源欄會保留一個分頁。選取的分頁與清單連成一片，雙欄時會標示作用中的窗格。資料夾和搜尋分頁有不同圖示，寬度足夠時會顯示關閉按鈕。
- **分頁與路徑快捷鍵更新。** `Ctrl+1～8` 跳到指定分頁，`Ctrl+9` 跳到最後一個，`Ctrl+PgUp`／`Ctrl+PgDn` 切換分頁，`Ctrl+Shift+T` 在原位置重開剛關閉的分頁並保留瀏覽紀錄。預設用 `Ctrl+\` 切換雙欄、`Ctrl+E` 或 `Alt+D` 編輯路徑。舊的 `Ctrl+Shift+D`、`Ctrl+L` 會提示新按法；游標停在路徑列空白處也會顯示提示。
- **搜尋可依日期與大小篩選。** 資料夾清單與搜尋結果都支援預設及自訂範圍。例如在「下載」選「文件」和「本週」，搜尋框留白也能按 Enter 搜尋。條件變更時直接篩選現有結果，先前結果被截斷時才重新搜尋。狀態列會顯示結果數與已套用的條件，可逐項或一次清除。
- **搜尋框支援組合條件。** 可混用多個詞、完整詞組、排除詞、萬用字元、副檔名、大小、日期及檔案類型，例如 `"年度報告" ext:pdf;docx -草稿`、`size:1mb..10mb`、`dm:7d`、`folder:`。也支援日期區間、`datemodified:`、`file:` 和全形冒號。條件會先於 50,000 筆結果上限套用。搜尋框展開後標準尺寸最寬約 560 像素，並隨字級調整。
- **搜尋結果依相關性排序。** 檔名完全相同的最前面，接著是檔名開頭、詞首及其他位置命中；同級時路徑較淺、修改時間較新的優先。結果陸續加入時會保留選取與捲動位置。可點欄位改排序，或從「排序 › 相關性」切回；離開搜尋時會恢復資料夾原本的排序。
- **路徑列可以直接瀏覽 NAS 共用。** 輸入 `\\nas` 就能列出共用資料夾，點麵包屑上的電腦名稱即可回到清單。需要認證時會開啟 Windows 登入視窗，可記住認證，登入後繼續開啟。找不到主機、帳號衝突或訪客存取遭封鎖時會分別說明。
- **新增八組主題並整理設定。** 陶土、鼠尾草、海港、薰衣草、玫瑰、琥珀、湖水綠與石墨都有淺色和深色，可在「主題與顏色」預覽。既有自訂重點色與背景色會保留，兩者可個別重設。「文字與語言」集中管理字級與介面語言，側欄底部的個人化卡片及頂列深淺色按鈕已移除，讓資料夾樹延伸到底部。
- **詳細資料檢視留出框選空間。** 縮窄「所在位置」欄，在清單右側留下約六分之一寬度，方便點空白取消選取或拖曳框選。路徑被截斷時，停留在該格即可查看完整位置。
- **快速預覽整合為一個開關。** 圖片、影片與 PDF 統一由「啟用快速預覽」控制，可選滑鼠懸停或空白鍵觸發。關閉後仍可從右鍵選單手動預覽。
- **移動、刪除與復原。** 剪下貼上或拖放移動省去額外確認；移至資源回收筒則遵循 Windows 的刪除確認設定。可復原的操作完成後會提示 `Ctrl+Z`。`Shift+Delete` 或按住 Shift 點「刪除」會永久刪除，操作前一定確認，預設焦點在「取消」，執行後無法復原。
- **說明改為獨立視窗。** 按 `F1` 或右上角的 `?` 可一邊看說明一邊操作。可調整視窗大小與捲動，分頁涵蓋快速鍵、搜尋、快速預覽、檔案與資料夾、檢視與版面、關於。快速鍵一覽會反映目前設定；搜尋時按 `F1` 直接開啟語法表。`Ctrl+C` 複製整頁，`Esc` 關閉。
- **內部整理。** 將主視窗的分頁、導覽、搜尋、檔案操作、設定與鍵盤事件拆成獨立模組。主視窗、檔案選擇器與說明共用同一份快速鍵表，自訂綁定儲存在設定檔的 `Shortcuts` 欄位。

## 下載

- **StillfeatherSetup.exe**：安裝或升級用。已安裝且啟用更新提醒的使用者會收到通知。
- **RestoreExplorer.cmd**：Stillfeather 打不開時，可用它把資料夾、Win+E 與開啟檔案視窗切回 Windows。
- **update.json、update.json.sig**：供程式檢查及驗證更新，無須手動下載。

---

## New features

- **Detach a tab into its own window.** Drop it outside the window, or drag it down into the file list when maximized. The source tab closes after the destination loads its folder. Dragging the only tab in a single-pane window moves that window. Session restore remembers the main window’s tabs.
- **Organize Favorites into bookmark groups.** Right-click Favorites to create a group, then drop folders onto it to pin them. Move existing bookmarks between groups or rename a group from its context menu. Removing a group keeps its bookmarks in Favorites.
- **Customize keyboard shortcuts.** Open Settings › Shortcuts, choose a command, and enter one or more key combinations separated by a semicolon. Leave the field blank to disable the binding. A conflict prompt identifies affected commands. Changes take effect when you save.

## Fixes

- New folders are immediately selected for renaming, including folders created with Ctrl+Shift+N. Items created from the Windows New submenu are also selected for renaming. If a filter would hide the new folder, the list switches to All.
- When you move an open folder or its parent in another tab, switching back follows it to the new location. If the folder was deleted, the tab opens a surviving parent and explains the change. This also works for folders deleted by another application.
- Larger text now scales the path bar, subfolder menus, search box, filter chips, dialogs, and Help. Sort, View, Date, Size, tab context menus, and preview-mode menus resize their rows and follow the theme. File context menus with Windows shell extensions retain the system text size.
- The sidebar, cards, and text now adapt to custom backgrounds. Dark title bars also work on Windows 10 versions 1809–1909, and theme changes immediately update the main window and open dialogs.

## Improvements

- **Browser-style tabs.** Drag tabs to reorder them or move them between panes. An insertion line marks the destination and Esc cancels. Cross-pane moves leave a tab in the source pane. The selected tab blends into the file list, and an outline identifies the active pane. Folder and search tabs have distinct icons, and wider tabs show close buttons.
- **Tab and path shortcuts.** Ctrl+1–8 selects a numbered tab, Ctrl+9 selects the last, and Ctrl+PgUp/PgDn switches tabs. Ctrl+Shift+T reopens the last closed tab in its original position with its navigation history. By default, Ctrl+\ toggles dual panes and Ctrl+E or Alt+D edits the path. The old Ctrl+Shift+D and Ctrl+L bindings display reminders. Hover over empty space in the path bar for the edit hint.
- **Date and size filters.** Filter folder views and search results using presets or custom ranges. For example, choose Documents and This week in Downloads, leave the query blank, and press Enter. Changing a filter updates existing results immediately. If earlier results were truncated, Stillfeather runs the search again. The status line shows the result count and active filters, which you can clear individually or together.
- **Combine search conditions.** Use multiple words, exact phrases, exclusions, wildcards, extensions, sizes, dates, and item types. Try `"annual report" ext:pdf;docx -draft`, `size:1mb..10mb`, `dm:7d`, or `folder:`. Date ranges, `datemodified:`, `file:`, and full-width colons are also supported. Conditions apply before the 50,000-result limit. The search box expands to about 560 pixels at standard text size and scales with the font.
- **Results ranked by relevance.** Exact names come first, followed by name prefixes, word prefixes, and other matches. Shallower paths and newer files break ties. Incoming results preserve selection and scroll position. Click a column to change sorting, choose Sort › Relevance to switch back, or leave search to restore the folder’s sort order.
- **Browse NAS shares from the path bar.** Enter `\\nas` to list a computer’s shares. Click the computer breadcrumb to return to the list. Windows prompts for credentials when needed, can remember them, and continues opening the folder after sign-in. Errors distinguish an unreachable host, conflicting credentials, and blocked guest access.
- **Eight themes and reorganized settings.** Preview Clay, Sage, Harbor, Lavender, Rose, Amber, Teal, and Graphite in light and dark variants under Theme & colors. Existing accent and background overrides carry over and can be reset individually. Text & language contains text size and interface language. Removing the sidebar personalization card and top-bar theme toggle gives the folder tree the full sidebar height.
- **More room for box selection.** In Details view, the narrower Location column leaves about one sixth of the list width free. Click there to deselect or drag to box-select. Hover over a clipped location to read its full path.
- **One quick-preview switch.** Images, videos, and PDFs share the Enable quick preview setting. Choose hover or Space to trigger previews. The context-menu preview remains available when the switch is off.
- **Move, delete, and undo.** Cut-and-paste and drag-and-drop moves skip the extra confirmation. Recycle Bin confirmation follows Windows preferences. The status line highlights Ctrl+Z when undo is available. Shift+Delete or Shift-clicking Delete permanently deletes items after confirmation, with Cancel focused by default. Permanent deletion cannot be undone.
- **A separate Help window.** Press F1 or click ? to read Help alongside the main window. The resizable, scrollable window covers shortcuts, search, preview, files and folders, views and layout, and app information. Shortcuts reflect your current settings. F1 during search opens the syntax guide, Ctrl+C copies the page, and Esc closes Help.
- **Internal changes.** Tab management, navigation, search, file operations, settings, and keyboard handling now live in separate modules. The main window, file picker, and Help share one runtime keymap. Custom bindings are saved in the `Shortcuts` settings field.

## Downloads

- **StillfeatherSetup.exe** — the installer for new installations and upgrades. Installed copies notify you when update reminders are enabled.
- **RestoreExplorer.cmd** — restores Windows handling of folders, Win+E, and file dialogs if Stillfeather won’t open.
- **update.json** and **update.json.sig** — used to check and verify automatic updates. No manual download is needed.
