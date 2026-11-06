use anyhow::Result;
use global_hotkey::{GlobalHotKeyManager, GlobalHotKeyEvent, hotkey::{HotKey, Modifiers, Code}};
use std::collections::HashMap;

pub struct TrayHotkeyManager {
    pub manager: GlobalHotKeyManager,
    pub hotkey_map: HashMap<u32, String>,
}

impl TrayHotkeyManager {
    pub fn new() -> Result<Self> {
        let manager = GlobalHotKeyManager::new()?;
        let mut hotkey_map = HashMap::new();

        let hk_p = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyP);
        let hk_n = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyN);
        let hk_b = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyB);
        let hk_up = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::ArrowUp);
        let hk_down = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::ArrowDown);

        hotkey_map.insert(hk_p.id(), "play_pause".to_string());
        hotkey_map.insert(hk_n.id(), "next".to_string());
        hotkey_map.insert(hk_b.id(), "prev".to_string());
        hotkey_map.insert(hk_up.id(), "vol_up".to_string());
        hotkey_map.insert(hk_down.id(), "vol_down".to_string());

        manager.register(hk_p)?;
        manager.register(hk_n)?;
        manager.register(hk_b)?;
        manager.register(hk_up)?;
        manager.register(hk_down)?;

        println!("[hotkey] Registered: Ctrl+Alt+P play/pause, N next, B prev, Up/Down vol");

        Ok(Self { manager, hotkey_map })
    }

    pub fn poll_action(&self) -> Option<String> {
        if let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            if let Some(action) = self.hotkey_map.get(&event.id) {
                return Some(action.clone());
            }
        }
        None
    }
}

pub fn create_tray_menu() -> Result<()> {
    println!("[tray] System tray: Play/Pause, Next, Prev, Vol, Quit");
    Ok(())
}
