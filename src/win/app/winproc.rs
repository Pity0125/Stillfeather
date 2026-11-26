//! 主視窗的視窗程序：把 Windows 訊息分派給 `App` 的各個處理方法。

use super::*;

pub(super) unsafe extern "system" fn main_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(a) = app_opt() else { return DefWindowProcW(hwnd, msg, wparam, lparam) };
    if a.hwnd != hwnd {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    // 下拉選單（排序、檢視、日期、大小、分頁選單）的項目是自己畫的（字型跟著文字大小，見 menu.rs）
    if let Some(r) = crate::win::menu::owner_draw(msg, wparam, lparam) {
        return r;
    }
    // 右鍵選單中 Windows 項目的子選單（新增 ▸、傳送到 ▸）與自繪圖示
    if let Some(r) = crate::win::shellmenu::handle_menu_msg(msg, wparam, lparam) {
        return r;
    }
    // 檔案總管重新啟動了：重新登記到殼層視窗清單（見 shellwin.rs）
    if msg >= 0xC000 && msg == crate::win::shellwin::taskbar_created_msg() {
        crate::win::shellwin::on_taskbar_created();
    }
    match msg {
        WM_SIZE => {
            a.close_crumb_menu();
            a.relayout();
            a.reposition_quicklook();
            return 0;
        }
        WM_MOVE => {
            a.close_crumb_menu();
            a.reposition_quicklook();
            return 0;
        }
        WM_GETMINMAXINFO => {
            let mmi = &mut *(lparam as *mut MINMAXINFO);
            mmi.ptMinTrackSize = POINT { x: a.s(960), y: a.s(600) };
            return 0;
        }
        WM_ERASEBKGND => return 1,
        WM_PAINT => {
            a.paint();
            return 0;
        }
        WM_SETCURSOR => {
            if swallow_modal_click(hwnd, lparam) {
                return 1;
            }
            if wparam as HWND == hwnd {
                let mut p = cursor_pos();
                ScreenToClient(hwnd, &mut p);
                if a.drag.is_some() || a.splitter_at(p.x, p.y).is_some() {
                    SetCursor(load_cursor(IDC_SIZEWE));
                    return 1;
                }
                if a.footer_link.is_some_and(|r| r.contains(p.x, p.y)) {
                    SetCursor(load_cursor(IDC_HAND));
                    return 1;
                }
            }
        }
        WM_LBUTTONDOWN => {
            let (x, y) = (get_x_lparam(lparam), get_y_lparam(lparam));
            if let Some(d) = a.splitter_at(x, y) {
                a.drag = Some(d);
                SetCapture(hwnd);
                return 0;
            }
            if a.picker_click(x, y) {
                return 0;
            }
            if a.footer_link.is_some_and(|r| r.contains(x, y)) {
                a.clear_filters();
                return 0;
            }
            for i in 0..2 {
                if a.layout.panes[i].contains(x, y) {
                    a.activate(Side::from_index(i));
                }
            }
            return 0;
        }
        WM_MOUSEMOVE => {
            if a.drag.is_some() {
                a.drag_to(get_x_lparam(lparam));
            }
            return 0;
        }
        WM_LBUTTONUP => {
            if a.drag.is_some() {
                ReleaseCapture();
                a.end_drag();
            }
            return 0;
        }
        WM_CAPTURECHANGED => {
            a.end_drag();
            return 0;
        }
        WM_COMMAND => {
            a.command(loword(wparam), hiword(wparam), lparam as HWND);
            return 0;
        }
        WM_NOTIFY => {
            let hdr = &*(lparam as *const NMHDR);
            if let Some(side) = a.side_of_list(hdr.hwndFrom) {
                return a.list_notify(side, hdr.code, lparam);
            }
            if hdr.hwndFrom == a.tree {
                return a.tree_notify(hdr.code, lparam);
            }
        }
        WM_CONTEXTMENU => {
            let src = wparam as HWND;
            let pt = POINT { x: get_x_lparam(lparam), y: get_y_lparam(lparam) };
            if let Some(side) = a.side_of_list(src) {
                a.list_context_menu(side, pt, lparam == -1);
                return 0;
            }
            if src == a.tree {
                a.tree_context_menu(pt, lparam == -1);
                return 0;
            }
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
            let hdc = wparam as HDC;
            let p = a.theme.pal;
            let is_status = lparam as HWND == a.pane_status[0] || lparam as HWND == a.pane_status[1];
            SetTextColor(hdc, if is_status { p.muted } else { p.ink });
            SetBkColor(hdc, p.surface);
            return a.theme.brushes.surface;
        }
        WM_TIMER => {
            if wparam == preview::TIMER_ID {
                preview::on_timer(&mut a.preview, hwnd);
            } else if wparam == TIMER_DRIVES {
                KillTimer(hwnd, TIMER_DRIVES);
                a.refresh_drives();
            } else if wparam == TIMER_FOOTER_HOT {
                KillTimer(hwnd, TIMER_FOOTER_HOT);
                a.invalidate_footer();
            } else if wparam == preview::HOLD_TIMER {
                // 空白鍵已放開（放開的訊息可能被輸入法或其他視窗拿走）
                if preview::is_holding(&a.preview) && !preview::space_held() {
                    crate::win::crashlog::crumb("計時器：空白鍵已放開");
                    a.hide_preview();
                }
            } else if wparam == crate::win::pane::HOVER_TIMER {
                a.on_hover_timer();
            } else if wparam == crate::win::pane::GATE_TIMER {
                a.on_gate_timer();
            } else if wparam == preview::SHOW_TIMER {
                preview::on_show_timer(&mut a.preview, hwnd);
            } else if wparam == TIMER_SEARCH {
                a.on_search_timer();
            } else if wparam == TIMER_SHELL_REFRESH {
                KillTimer(hwnd, TIMER_SHELL_REFRESH);
                a.refresh_after_shell();
            } else if wparam == crate::win::update::TIMER_CHECK {
                a.on_update_timer();
            } else if wparam == crate::win::update::TIMER_PROMPT {
                a.try_update_prompt();
            } else if wparam == crate::win::watch::TIMER_WATCH {
                a.on_watch_timer();
            } else if wparam == crate::win::picker::TIMER_HOST {
                a.picker_host_timer();
            } else if wparam == crate::win::thumbs::TIMER_REDRAW {
                a.thumbs.redraw_due(hwnd);
                a.redraw_thumbnails();
            } else if wparam == crate::win::fileicons::TIMER_REDRAW {
                a.file_icons.redraw_due(hwnd);
                a.redraw_file_icons();
            }
            return 0;
        }
        crate::win::shellwin::WM_SHELL_SELECT => {
            let sel = Box::from_raw(lparam as *mut crate::win::shellwin::Select);
            a.shell_select(sel.path, sel.replace);
            return 0;
        }
        crate::win::shellwin::WM_SHELL_FRONT => {
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            if SetForegroundWindow(hwnd) == 0 {
                crate::win::shellwin::log("無法把視窗拉到前面（Windows 不允許）");
            }
            return 0;
        }
        crate::win::thumbs::WM_APP_THUMB => {
            let done = Box::from_raw(lparam as *mut crate::win::thumbs::Done);
            a.thumbs.on_result(hwnd, *done);
            return 0;
        }
        crate::win::fileicons::WM_APP_FILE_ICON => {
            let done = Box::from_raw(lparam as *mut crate::win::fileicons::Done);
            a.file_icons.on_result(hwnd, *done, &mut a.icons);
            return 0;
        }
        crate::win::watch::WM_APP_WATCH => {
            a.on_watch_message(wparam, lparam);
            return 0;
        }
        WM_APP_WORKER => {
            let msg = Box::from_raw(lparam as *mut WorkerMsg);
            match *msg {
                WorkerMsg::Dir { tab, generation, path, kind, result, gone_to } => a.on_dir(tab, generation, path, kind, result, gone_to),
                WorkerMsg::NotFolder { tab, generation, path, kind } => a.on_not_folder(tab, generation, path, kind),
                WorkerMsg::AutoDir { tab, generation, path, result, gone_to } => a.on_auto_dir(tab, generation, path, result, gone_to),
                WorkerMsg::SearchBatch { tab, generation, entries, scanned, skipped } => a.on_search_batch(tab, generation, entries, scanned, skipped),
                WorkerMsg::SearchDone { tab, generation, result } => a.on_search_done(tab, generation, result),
                WorkerMsg::Tree { item, token, result } => a.on_tree_children(item, token, result),
                WorkerMsg::CrumbMenu { token, result } => crumbmenu::on_loaded(&mut a.crumb_menu, token, result, &a.theme, a.icons.icon_px),
                WorkerMsg::Preview { generation, path, result } => {
                    crate::win::crashlog::crumb("主視窗：收到預覽結果");
                    preview::on_result(&mut a.preview, generation, &path, result)
                }
                WorkerMsg::FileOp(report) => a.on_file_op(report),
                WorkerMsg::Detached(id, generation, result) => a.finish_detach(id, generation, result),
                WorkerMsg::Update(event) => a.on_update_event(event),
            }
            return 0;
        }
        WM_APP_OPEN => {
            let paths = Box::from_raw(lparam as *mut Vec<String>);
            a.open_paths(*paths, false, false);
            return 0;
        }
        WM_APP_TREE_RESET => {
            a.reset_tree_node(lparam as HTREEITEM);
            return 0;
        }
        WM_APP_SEARCH => {
            a.sync_search();
            show(a.clear_btn, a.clear_shown());
            return 0;
        }
        WM_APP_RENAME => {
            let data = Box::from_raw(lparam as *mut (TabId, String, String));
            a.finish_rename(data.0, &data.1, &data.2);
            return 0;
        }
        WM_APP_BEGIN_RENAME => {
            let item = Box::from_raw(lparam as *mut NewItem);
            a.begin_rename_new(*item);
            return 0;
        }
        tabs::WM_APP_TAB => {
            a.tab_action(wparam, lparam);
            return 0;
        }
        chips::WM_APP_CHIP => {
            a.chip_action(wparam);
            return 0;
        }
        crumbs::WM_APP_CRUMB => {
            a.crumb_action(wparam);
            return 0;
        }
        crumbs::WM_APP_CRUMB_MENU => {
            a.crumb_menu_action(wparam, lparam != 0);
            return 0;
        }
        WM_COPYDATA => {
            return match instance::decode_copydata(lparam) {
                Some(paths) => {
                    let p = Box::into_raw(Box::new(paths));
                    if PostMessageW(hwnd, WM_APP_OPEN, 0, p as LPARAM) == 0 {
                        drop(Box::from_raw(p));
                        0
                    } else {
                        1
                    }
                }
                None => 0,
            };
        }
        WM_DEVICECHANGE => {
            if matches!(wparam, DBT_DEVICEARRIVAL | DBT_DEVICEREMOVECOMPLETE | 7) {
                SetTimer(hwnd, TIMER_DRIVES, 500, std::ptr::null());
            }
            return 1;
        }
        WM_SYSCOMMAND => {
            // Alt＋字母（或單按 Alt、F10）在沒有選單列的視窗會發出提示音；只保留 Alt+空白鍵的系統選單。
            if wparam & 0xFFF0 == SC_KEYMENU && lparam != b' ' as LPARAM {
                return 0;
            }
        }
        WM_SETTINGCHANGE => {
            anim::refresh_settings();
            if lparam != 0 && from_wide_ptr(lparam as *const u16) == "ImmersiveColorSet" {
                a.system_theme_changed();
            }
        }
        WM_THEMECHANGED => {
            a.system_theme_changed();
        }
        WM_DPICHANGED => {
            let r = *(lparam as *const RECT);
            a.on_dpi_changed(loword(wparam) as u32, &r);
            return 0;
        }
        WM_ACTIVATE => {
            if loword(wparam) == 0 {
                a.close_crumb_menu();
                a.hide_preview();
                let f = GetFocus();
                if f != 0 && IsChild(hwnd, f) != 0 {
                    a.last_focus = f;
                }
            } else if hiword(wparam) == 0 {
                a.reload_shared_preferences();
                a.restore_focus();
                return 0;
            }
        }
        WM_SETFOCUS => {
            a.restore_focus();
            return 0;
        }
        WM_CLOSE => {
            crate::win::crashlog::crumb("主視窗：WM_CLOSE");
            if a.op_busy {
                dialog::info(hwnd, tr!("檔案作業還在進行", "A file operation is still running"), tr!("請等它完成，或在 Windows 的進度視窗按取消後再關閉 Stillfeather。", "Wait for it to finish, or click Cancel in the Windows progress window, then close Stillfeather."));
                return 0;
            }
            if a.picker_mode() {
                a.picker_finish(feather_core::picker::EXIT_CANCEL);
                return 0;
            }
            a.hide_preview();
            // 「還原上次的分頁」：記下現在的分頁
            a.save_session();
            DestroyWindow(hwnd);
            return 0;
        }
        WM_ENDSESSION => {
            // Windows 登出、關機或重新啟動：不會收到 WM_CLOSE，在這裡記下分頁
            if wparam != 0 {
                a.save_session();
            }
            return 0;
        }
        WM_DESTROY => {
            crate::win::crashlog::crumb("主視窗：WM_DESTROY");
            crate::win::shellwin::stop();
            a.stop_watching();
            for h in [a.lists[0], a.lists[1], a.tree, a.crumbs, a.strips[0], a.strips[1]] {
                dragdrop::revoke(h);
            }
            for t in a.tabs.values() {
                t.cancel.store(true, Ordering::Relaxed);
            }
            if a.brand_icon != 0 {
                DestroyIcon(a.brand_icon);
                a.brand_icon = 0;
            }
            if a.wordmark.0 != 0 {
                DestroyIcon(a.wordmark.0);
                a.wordmark.0 = 0;
            }
            // 選擇器：結束代碼告訴取代元件是確定、取消還是改用 Windows 的視窗
            PostQuitMessage(a.picker.as_ref().map_or(0, |p| p.exit_code));
            return 0;
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}
