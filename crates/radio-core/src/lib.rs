use radio_api::{RadioBrowserClient, Station, SearchFilters};
use radio_player::{Player, PlayerState};
use radio_storage::Storage;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

pub struct RadioAppCore {
    api: RadioBrowserClient,
    player: Mutex<Player>,
    storage: Storage,
    current_station: Option<Station>,
    current_station_name: String,
}

impl RadioAppCore {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            api: RadioBrowserClient::new(),
            player: Mutex::new(Player::new().map_err(|e| anyhow::anyhow!(e))?),
            storage: Storage::new().map_err(|e| anyhow::anyhow!(e))?,
            current_station: None,
            current_station_name: "No station".to_string(),
        })
    }

    pub async fn search(&self, q: &str, country: Option<&str>, genre: Option<&str>) -> anyhow::Result<Vec<Station>> {
        Ok(self.api.search(q, country, genre, 50).await?)
    }

    pub async fn search_with_limit(&self, q: &str, country: Option<&str>, genre: Option<&str>, limit: usize) -> anyhow::Result<Vec<Station>> {
        Ok(self.api.search(q, country, genre, limit).await?)
    }

    pub async fn search_filtered(&self, filters: SearchFilters) -> anyhow::Result<Vec<Station>> {
        Ok(self.api.search_filtered(filters).await?)
    }

    pub fn play_station(&mut self, station: Station) -> anyhow::Result<()> {
        let url = if !station.url_resolved.is_empty() { station.url_resolved.clone() } else { station.url.clone() };
        self.current_station_name = station.name.clone();
        self.player.lock().unwrap().play(&url).map_err(|e| anyhow::anyhow!(e))?;
        self.storage.add_history(&station).ok();
        self.current_station = Some(station);
        Ok(())
    }

    pub fn stop(&mut self) {
        self.player.lock().unwrap().stop();
        self.current_station_name = "Stopped".to_string();
    }

    pub fn set_volume(&mut self, v: f32) { self.player.lock().unwrap().set_volume(v); }
    pub fn state(&self) -> PlayerState { 
        let mut p = self.player.lock().unwrap();
        p.state()
    }
    pub fn is_playing(&self) -> bool { 
        let mut p = self.player.lock().unwrap();
        matches!(p.state(), PlayerState::Playing) 
    }
    pub fn vu_level(&self) -> f32 { self.player.lock().unwrap().vu_level() }
    pub fn vu_peak(&self) -> f32 { self.player.lock().unwrap().vu_peak() }
    pub fn current_station_name(&self) -> String { self.current_station_name.clone() }
    pub fn is_recording(&self) -> bool { self.player.lock().unwrap().is_recording() }
    pub fn start_recording(&mut self, path: PathBuf) -> anyhow::Result<()> { self.player.lock().unwrap().start_recording(path).map_err(|e| anyhow::anyhow!(e)) }
    pub fn stop_recording(&mut self) { self.player.lock().unwrap().stop_recording(); }
    pub fn set_sleep_timer(&mut self, mins: u64) { self.player.lock().unwrap().set_sleep_timer(mins); }
    pub fn clear_sleep_timer(&mut self) { self.player.lock().unwrap().clear_sleep_timer(); }
    pub fn sleep_remaining(&self) -> Option<Duration> { self.player.lock().unwrap().sleep_remaining() }
    pub fn list_favorites(&self) -> anyhow::Result<Vec<Station>> { Ok(self.storage.list_favorites()?) }
    pub fn add_favorite(&self, station: &Station) -> anyhow::Result<()> { Ok(self.storage.add_favorite(station)?) }
    pub fn remove_favorite(&self, uuid: &str) -> anyhow::Result<()> { Ok(self.storage.remove_favorite(uuid)?) }
    pub fn is_favorite(&self, uuid: &str) -> bool { self.storage.is_favorite(uuid).unwrap_or(false) }
    pub fn list_history(&self) -> anyhow::Result<Vec<(Station, i64)>> { Ok(self.storage.list_history()?) }
    pub fn clear_history(&self) -> anyhow::Result<()> { Ok(self.storage.clear_history()?) }
    pub fn play_next_favorite(&mut self) -> anyhow::Result<()> {
        let favs = self.list_favorites()?;
        if favs.is_empty() { anyhow::bail!("No favorites"); }
        self.play_station(favs[0].clone())
    }
    pub fn play_prev_favorite(&mut self) -> anyhow::Result<()> {
        let favs = self.list_favorites()?;
        if favs.is_empty() { anyhow::bail!("No favorites"); }
        self.play_station(favs.last().unwrap().clone())
    }
}
