//! systems:audio. Fortnite's own sounds (WAV, converted from the player's install by the setup tool),
//! played through XAudio2 next to Elden Ring's Wwise output, with distance falloff for world sounds.
use crate::generated::FORTNITE_ASSETS;
use glam::Vec3;
use std::collections::HashMap;
use std::sync::Mutex;
use windows::Win32::Media::Audio::XAudio2::{
    IXAudio2, IXAudio2MasteringVoice, IXAudio2SourceVoice, XAUDIO2_BUFFER, XAUDIO2_DEFAULT_PROCESSOR, XAUDIO2_END_OF_STREAM,
    XAudio2CreateWithVersionInfo,
};
use windows::Win32::Media::Audio::{AudioCategory_GameEffects, WAVEFORMATEX};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

struct Sound {
    format: WAVEFORMATEX,
    data: Vec<u8>,
}

struct Engine {
    xa: IXAudio2,
    _master: IXAudio2MasteringVoice,
    sounds: HashMap<usize, Option<Sound>>,
    voices: Vec<IXAudio2SourceVoice>,
    listener: Vec3,
}

// XAudio2 objects are free-threaded.
unsafe impl Send for Engine {}

static ENGINE: Mutex<Option<Engine>> = Mutex::new(None);

pub fn init() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let mut xa: Option<IXAudio2> = None;
        if XAudio2CreateWithVersionInfo(&mut xa, 0, XAUDIO2_DEFAULT_PROCESSOR, 0).is_err() {
            crate::log!("audio: XAudio2 unavailable");
            return;
        }
        let Some(xa) = xa else { return };
        let mut master: Option<IXAudio2MasteringVoice> = None;
        if xa.CreateMasteringVoice(&mut master, 0, 0, 0, None, None, AudioCategory_GameEffects).is_err() {
            crate::log!("audio: no mastering voice");
            return;
        }
        *ENGINE.lock().unwrap() = Some(Engine { xa, _master: master.unwrap(), sounds: HashMap::new(), voices: Vec::new(), listener: Vec3::ZERO });
        crate::log!("audio: ready");
    }
}

/// Minimal RIFF/WAVE reader (PCM 16-bit, as the setup tool writes).
fn load(asset: usize) -> Option<Sound> {
    let path = crate::paths::cache_dir().join(FORTNITE_ASSETS[asset].out);
    let b = std::fs::read(&path).ok()?;
    if b.len() < 44 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return None;
    }
    let mut i = 12;
    let mut format = None;
    let mut data = None;
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let len = u32::from_le_bytes(b[i + 4..i + 8].try_into().ok()?) as usize;
        let body = b.get(i + 8..i + 8 + len)?;
        if id == b"fmt " && len >= 16 {
            let u16at = |o: usize| u16::from_le_bytes([body[o], body[o + 1]]);
            let u32at = |o: usize| u32::from_le_bytes([body[o], body[o + 1], body[o + 2], body[o + 3]]);
            format = Some(WAVEFORMATEX {
                wFormatTag: u16at(0),
                nChannels: u16at(2),
                nSamplesPerSec: u32at(4),
                nAvgBytesPerSec: u32at(8),
                nBlockAlign: u16at(12),
                wBitsPerSample: u16at(14),
                cbSize: 0,
            });
        } else if id == b"data" {
            data = Some(body.to_vec());
        }
        i += 8 + len + (len & 1);
    }
    Some(Sound { format: format?, data: data? })
}

pub fn set_listener(pos: Vec3) {
    if let Some(e) = ENGINE.lock().unwrap().as_mut() {
        e.listener = pos;
    }
}

/// Plays a fortnite_assets sound; `at` = world position for falloff (None = UI / your own gun).
pub fn play(asset: usize, at: Option<Vec3>) {
    let mut g = ENGINE.lock().unwrap();
    let Some(e) = g.as_mut() else { return };
    let volume = match at {
        Some(p) => (1.0 - p.distance(e.listener) / 60.0).clamp(0.0, 1.0).powi(2),
        None => 1.0,
    };
    if volume <= 0.01 {
        return;
    }
    if !e.sounds.contains_key(&asset) {
        let s = load(asset);
        if s.is_none() {
            crate::log!("audio: {} missing", FORTNITE_ASSETS[asset].id);
        }
        e.sounds.insert(asset, s);
    }
    let Some(Some(sound)) = e.sounds.get(&asset) else { return };
    // reap finished voices
    e.voices.retain(|v| unsafe {
        let mut st = Default::default();
        v.GetState(&mut st, 0);
        if st.BuffersQueued == 0 { v.DestroyVoice(); false } else { true }
    });
    if e.voices.len() > 48 {
        return;
    }
    unsafe {
        let mut voice: Option<IXAudio2SourceVoice> = None;
        if e.xa.CreateSourceVoice(&mut voice, &sound.format, 0, 2.0, None, None, None).is_err() {
            return;
        }
        let Some(voice) = voice else { return };
        let buf = XAUDIO2_BUFFER {
            Flags: XAUDIO2_END_OF_STREAM,
            AudioBytes: sound.data.len() as u32,
            pAudioData: sound.data.as_ptr(),
            ..Default::default()
        };
        let _ = voice.SetVolume(volume, 0);
        if voice.SubmitSourceBuffer(&buf, None).is_ok() && voice.Start(0, 0).is_ok() {
            e.voices.push(voice);
        } else {
            voice.DestroyVoice();
        }
    }
}
