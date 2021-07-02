use std::io::{BufReader, prelude::*};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError::*};
use std::collections::HashMap;
use std::net::TcpStream;
use std::thread::spawn;
use std::sync::RwLock;

use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::pixels::Color;
use sdl2::render::BlendMode;
use sdl2::rect::{Point, Rect};

use rand::random;
use serde::{Deserialize, Serialize};

use rusttype::{point, Font, Scale};

use reqwest::blocking::Client;

use image::RgbaImage;
use image::imageops::FilterType::Lanczos3;

use fancy_regex::Regex;
use lazy_static::lazy_static;

lazy_static! {
    static ref REIRC: Regex = Regex::new(r"^(?:@([^ ]+) )?(?:[:](\S+) )?(\S+)(?: (?!:)(.+?))?(?: [:](.+))?$").unwrap();
    static ref RETAGS: Regex = Regex::new(r"([^=;]+)=([^;]*)").unwrap();
    static ref REBADGES: Regex = Regex::new(r"([^,]+)\/([^,]*)").unwrap();
    static ref REEMOTES: Regex = Regex::new(r"([^\/]+):([^\/]*)").unwrap();
    static ref REEMOTEINDEX: Regex = Regex::new(r"([^,]+)-([^,]*)").unwrap();
    static ref REACTION: Regex = Regex::new(r"^\u0001ACTION (.*)\u0001$").unwrap();
    static ref REHOST: Regex = Regex::new(r"([a-z_0-9]+)!([a-z_0-9]+)@([a-z._0-9]+)").unwrap();

    static ref CLIENT: Client = Client::new();

    static ref CACHE: RwLock<HashMap<String, Vec<u8>>> = RwLock::new(HashMap::new());
    static ref EMOTES: RwLock<HashMap<String, Vec<(String, String, EmoteType)>>> = RwLock::new(HashMap::new());
    static ref GLOBALS: Vec<(String, String, EmoteType)> = {
        let mut v = Vec::new();
        let bttv: Vec<BttvEmote> = CLIENT.get("https://api.betterttv.net/3/cached/emotes/global")
            .send().unwrap().json().unwrap();
        for e in bttv {
            v.push((e.code.to_owned(), e.id.to_owned(), EmoteType::BTTV));
        }
        v
    };
}

pub fn main() -> Result<()> {
    let (s, r) = channel();
    let (e, _r) = channel();
    let emotes = e.clone();

    spawn(move|| {
        Ok::<(), Error<String>>(
            twitch(s, e, vec![
                "hjamu",
                "xqcow",
            ])?
        )
    });
    spawn(move|| {
        Ok::<(), Error<String>>(
            emotes_thread(_r)?
        )
    });
    
    let sdl_context = sdl2::init()?;
    let video_subsystem = sdl_context.video()?;

    let mut width = 1200;
    let mut height = 600;

    let mut tabs = Tabs::new();
    let tabs_offset = 0;
    let font = Font::try_from_bytes(include_bytes!("font.ttf")).unwrap();
    let scale = Scale::uniform(20.0);
    let v_metrics = font.v_metrics(scale);

    let window = video_subsystem
        .window("chatJAM", width, height)
        .position_centered()
        .resizable()
        .allow_highdpi()
        .opengl()
        .build()
        .map_err(|e| e.to_string())?;

    let mut canvas = window.into_canvas().build().map_err(|e| e.to_string())?;
    canvas.set_blend_mode(BlendMode::Blend);

    unsafe {
        // I know I'm supposed to avoid unsafe code, but I couldn't find how to do this with the actual crate
        sdl2::sys::SDL_SetWindowMinimumSize(canvas.window_mut().raw(), 800, 300);
    }

    let mut event_pump = sdl_context.event_pump()?;

    'running: loop {
        for event in event_pump.poll_iter() {
            match event {
                Event::Quit { .. }
                | Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => break 'running,
                Event::KeyDown {
                    keycode: Some(Keycode::Tab),
                    ..
                } => {
                    tabs.next();
                    let s = format!("chatJAM - {}", tabs.current().unwrap_or(String::from("None")));
                    canvas.window_mut().set_title(&s).unwrap();
                },
                _ => {}
            }
        }
        let size = canvas.window_mut().size();
        width = size.0;
        height = size.1;

        canvas.set_draw_color(Color {r: 38, g: 38, b: 34, a: 255});
        canvas.clear();

        match tabs.channel() {
            Some(v) => {
                let mut prev: Option<String> = None;
                let mut lines = 1;
                for msg in v.iter().rev() {
                    if lines as f32 * v_metrics.ascent >= (height + 100) as f32 {
                        break;
                    }
                    let mut pixels = Vec::new();
                    let line = msg.build_glyphs(emotes.clone(), &mut pixels, &font, scale, width) as u32;
                    lines += line;
                    let same = if let Some(p) = prev.clone() {
                        if msg.user != p.to_owned() {
                            prev = Some(msg.user.to_owned());
                            false
                        } else {
                            true
                        }
                    } else {
                        prev = Some(msg.user.to_owned());
                        false
                    };
                    let y = height - (lines - 1) * v_metrics.ascent as u32;
                    for pixel in pixels {
                        canvas.set_draw_color(pixel.c);
                        canvas.draw_point(Point::new(5 + pixel.x as i32, y as i32 + pixel.y as i32))?;
                    }
                    if !same {
                        let y = y + (line - 1) * v_metrics.ascent as u32;
                        canvas.set_draw_color(Color {r: 78, g: 78, b: 84, a: 255});
                        canvas.draw_line(Point::new(5, y as i32 + 3), Point::new(width as i32 - 5, y as i32 + 3))?;
                    }
                }
                if tabs.channels.len() > 1 {
                    canvas.set_draw_color(Color {r: 78, g: 78, b: 84, a: 255});
                    canvas.fill_rect(Rect::new(0, 0, width, 25)).unwrap();
                    let mut x = tabs_offset;
                    for (i, c) in tabs.channels.iter().enumerate() {
                        let glyphs: Vec<_> = font.layout(&c, scale, point(x as f32 + 5.0, v_metrics.ascent)).collect();
                        let width = {
                            let min_x = glyphs
                                .first()
                                .map(|g| g.pixel_bounding_box().unwrap().min.x)
                                .unwrap();
                            let max_x = glyphs
                                .last()
                                .map(|g| g.pixel_bounding_box().unwrap().max.x)
                                .unwrap();
                            (max_x - min_x) as u32
                        };
                        if i == tabs.index {
                            canvas.set_draw_color(Color {r: 38, g: 38, b: 44, a: 255});
                            canvas.fill_rect(Rect::new(x as i32, 0, width + 10, 25)).unwrap();
                        }
                        x += width + 10;
                        for glyph in &glyphs {
                            if let Some(bounding_box) = glyph.pixel_bounding_box() {
                                glyph.draw(|x, y, v| {
                                    let x = x as i32 + bounding_box.min.x;
                                    let y = y as i32 + bounding_box.min.y;
                                    canvas.set_draw_color(Color {r: 255, g: 255, b: 255, a: (v * 255.0) as u8});
                                    canvas.draw_point(Point::new(x, y)).unwrap();
                                });
                            }
                        }
                    }
                }
            },
            _ => {},
        }

        loop {
            match r.try_recv() {
                Ok(mut msg) => {
                    if !msg.room.starts_with("#") {
                        // Most likely a whisper?
                        msg.room = msg.user.to_owned();
                    }
                    match msg.command.as_ref() {
                        "JOIN" | "PART" | "PRIVMSG" | "WHISPER" | "HOSTTARGET" |
                        "NOTICE" | "USERNOTICE" | "RECONNECT" => {
                            tabs.add(msg)?;
                        },
                        "CLEARCHAT" | "CLEARMSG" => {
                            // TODO 
                        },
                        _ => println!("{}", msg.raw),
                    }
                },
                Err(Empty) => break,
                Err(Disconnected) => break,
            }
        }

        canvas.present();
    }

    Ok(())
}

fn twitch(sender: Sender<Message>, emotes: Sender<Emotes>, channels: Vec<&str>) -> Result<()> {
    let mut stream = TcpStream::connect("irc.twitch.tv:6667")?;
    stream.write(b"USER irc.twitch.tv\r\n")?;

    stream.write(b"PASS nopass\r\n")?;
    stream.write(format!("NICK justinfan{}\r\n", random::<u32>()).as_bytes())?;
    stream.write(b"CAP REQ :twitch.tv/commands\r\n")?;
    stream.write(b"CAP REQ :twitch.tv/tags\r\n")?;
    for channel in channels {
        stream.write(format!("JOIN #{}\r\n", channel).as_bytes())?;
    }
    let mut o = stream.try_clone()?;
    let mut lines = BufReader::new(&mut o).lines();

    while let Some(line) = lines.next() {
        println!("{:?}", &line);
        for res in REIRC.captures_iter(&line?) {
            match res {
                Ok(c) => {
                    let mut msg = Message::default();
                    msg.raw = match c.get(0) {
                        Some(x) => x.as_str(),
                        None => "",
                    }.to_owned();
                    let tag_data = match c.get(1) {
                        Some(x) => x.as_str(),
                        None => "",
                    };
                    msg.user = match c.get(2) {
                        Some(x) => {
                            let res = REHOST.captures(x.as_str()).unwrap(); 
                            match res.iter().next() {
                                Some(c) => match c.get(1) {
                                    Some(x) => x.as_str(),
                                    None => "",
                                },
                                None => "",
                            }
                        },
                        None => "",
                    }.to_owned();
                    msg.command = match c.get(3) {
                        Some(x) => x.as_str(),
                        None => "",
                    }.to_owned();
                    msg.room = match c.get(4) {
                        Some(x) => x.as_str(),
                        None => "",
                    }.to_owned();
                    msg.message = if let Some(x) = c.get(5) {
                        let res = REACTION.captures(x.as_str()).unwrap();
                        if let Some(c) = res.iter().next() {
                            if let Some(d) = c.get(1) {
                                msg.action = true;
                                d.as_str()
                            } else {
                                msg.action = false;
                                x.as_str()
                            }
                        } else { x.as_str() }
                    } else { "" }.to_owned();
                    for res in RETAGS.captures_iter(tag_data) {
                        match res {
                            Ok(c) => {
                                let key = c.get(1).unwrap().as_str();
                                let val = c.get(2).unwrap().as_str();
                                match key {
                                    "badges" | "badge-info" => for res in REBADGES.captures_iter(val) {
                                        if let Ok(c) = res {
                                            let badge = c.get(1).unwrap().as_str();
                                            let tier = c.get(2).unwrap().as_str();
                                            if key == "badges" {
                                                msg.tags.badges.insert(badge.to_owned(), tier.to_owned());
                                            }
                                            if key == "badge-info" {
                                                msg.tags.badge_info.insert(badge.to_owned(), tier.to_owned());
                                            }
                                        }
                                    },
                                    "emotes" => for res in REEMOTES.captures_iter(val) {
                                        if let Ok(c) = res {
                                            let emoteid = c.get(1).unwrap().as_str();
                                            let indices = c.get(2).unwrap().as_str();
                                            for res in REEMOTEINDEX.captures_iter(indices) {
                                                if let Ok(c) = res {
                                                    let start = c.get(1).unwrap().as_str().parse().unwrap();
                                                    let end = c.get(2).unwrap().as_str().parse().unwrap();
                                                    let emote: String = msg.message.chars().skip(start).take(end - start).collect();
                                                    if let Some(emotes) = msg.tags.emotes.get_mut(&emote) {
                                                        emotes.push((
                                                            emoteid.to_owned(),
                                                            start, end,
                                                        ));
                                                    } else {
                                                        msg.tags.emotes.insert(
                                                            emote.to_owned(),
                                                            vec![(
                                                                emoteid.to_owned(),
                                                                start, end,
                                                            )]
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                    },
                                    _ => msg.tags.set(key, val),
                                }
                            },
                            Err(_) => {},
                        }
                    }
                    match msg.command.as_ref() {
                        "PING" => { stream.write(b"PONG tmi.twitch.tv\r\n")?; },
                        "ROOMSTATE" => emotes.send(Emotes::room(&msg.tags.room_id)).unwrap(),
                        // We ignore these
                        "HOSTTARGET" => {},
                        _ => sender.send(msg).unwrap(),
                    }
                }
                Err(_) => {},
            }

        }
    }

    Ok(())
}


fn emotes_thread(r: Receiver<Emotes>) -> Result<()> {
    loop {
        match r.try_recv() {
            Ok(emote) => match emote.kind {
                EmoteType::ROOM => {
                    get_channel_emotes(&emote.id, EmoteType::BTTV);
                    get_channel_emotes(&emote.id, EmoteType::FFZ);
                },
                _ => download_emote(emote),
            },
            Err(Empty) => continue,
            Err(Disconnected) => break,
        }
    }
    Ok(())
}

fn get_channel_emotes(id: &str, e: EmoteType) {
    let bttv = match e {
        EmoteType::BTTV => true,
        _ => false,
    };

    let url = if bttv {
        format!("https://api.betterttv.net/3/cached/users/twitch/{}", id)
    } else {
        format!("https://api.betterttv.net/3/cached/frankerfacez/users/twitch/{}", id)
    };
    let channel = if bttv {
        let channel: BttvChannel = CLIENT.get(&url).send().unwrap().json().unwrap();
        channel.emotes()
    } else {
        let channel: Vec<FfzEmote> = CLIENT.get(&url).send().unwrap().json().unwrap();
        channel.iter().map(|x| Emotes::emote(
            &format!("{}", x.id), 
            &x.code, 
            EmoteType::FFZ
        )).collect()
    };
    for emote in channel {
        if let Ok(ref mut emotes) = EMOTES.write() {
            if let Some(room) = emotes.get_mut(id) {
                room.push((emote.code.to_owned(), emote.id.to_owned(), e));
            } else {
                emotes.insert(id.to_owned(), vec![(emote.code.to_owned(), emote.id.to_owned(), e)]);
            }
        }
        //download_emote(emote.clone());
    }
}

fn download_emote(e: Emotes) {
    if let Ok(ref cache) = CACHE.read() {
        if let Some(_) = cache.get(&e.code) {
            return;
        }
    }
    let url = match e.kind {
        EmoteType::TTV => format!("https://static-cdn.jtvnw.net/emoticons/v1/{}/1.0", e.id),
        EmoteType::BTTV => format!("https://cdn.betterttv.net/emote/{}/1x", e.id),
        EmoteType::FFZ => format!("https://cdn.frankerfacez.com/emote/{}/1", e.id),
        EmoteType::ROOM => String::new(),
    };
    if let Ok(res) = CLIENT.get(url).send() {
        if let Ok(b) = res.bytes() {
            if let Ok(ref mut cache) = CACHE.write() {
                println!("Caching {}: {} of type {:?}", e.code, e.id, match e.kind {
                    EmoteType::TTV => "EmoteType::TTV",
                    EmoteType::BTTV => "EmoteType::BTTV",
                    EmoteType::FFZ => "EmoteType::FFZ",
                    EmoteType::ROOM => "EmoteType::ROOM",
                });
                cache.insert(e.code.to_owned(), b.to_vec());
            }
        }
    }
}

// Spawns a thread to download emote, assumes id actually exists.
fn get_emote(s: Sender<Emotes>, e: Emotes, resize: u32) -> RgbaImage {
    if let Ok(ref mut cache) = CACHE.try_read() {
        if let Some(b) = cache.get(&e.code) {
            match image::load_from_memory(&b) {
                Ok(img) => return img.resize(400, resize, Lanczos3).to_rgba8(),
                Err(_) => return RgbaImage::from_pixel(resize, resize, *image::Pixel::from_slice(&[0, 0, 0, 255])),
            }
        } else {
            s.send(e).unwrap();
        }
    }
    RgbaImage::from_pixel(resize, resize, *image::Pixel::from_slice(&[0, 0, 0, 255]))
}

struct Tabs {
    open: Option<String>,
    index: usize,
    channels: Vec<String>,
    ids: Vec<String>,
    messages: HashMap<String, Vec<Message>>,
}

impl Tabs {
    fn new() -> Self {
        Self {
            open: None,
            index: 0,
            channels: Vec::new(),
            ids: Vec::new(),
            messages: HashMap::new(),
        }
    }

    fn current(&self) -> Option<String> {
        match &self.open {
            Some(s) => Some(s.to_owned()),
            None => None,
        }
    }

    fn next(&mut self) {
        if self.channels.len() > 0 {
            self.index += 1;
            self.index %= self.channels.len();
            self.open = Some(self.channels[self.index].to_owned());
        }
    }

    fn channel(&self) -> Option<&Vec<Message>> {
        match &self.open {
            Some(c) => self.messages.get(c),
            None => None,
        }
    }

    fn add(&mut self, msg: Message) -> Result<()> {
        let c = &msg.room;
        if !self.channels.contains(c) {
            self.channels.push(c.to_owned());
            self.ids.push(msg.tags.room_id.to_owned());
            if self.channels.len() > 0 {
                self.index += 1;
                self.index %= self.channels.len();
            }
            self.open = Some(c.to_owned());
        }
        Ok(self.messages.entry(c.to_owned()).or_insert(Vec::new()).push(msg.clone()))
    }
}

#[derive(Clone, Default)]
struct Message {
    tags: Tags,
    command: String,
    room: String,
    user: String,
    message: String,
    action: bool,
    raw: String,
}

impl Message {
    /// Sets a layout starting at (0, 0), wraps text.
    fn build_glyphs(&self, s: Sender<Emotes>, pixels: &mut Vec<Pixel>, font: &Font, scale: Scale, width: u32) -> usize {
        let mut lines = 1;
        match self.command.as_ref() {
            "JOIN" => {
                let s = format!("JOINED {}", self.room);
                let s: Vec<_> = font.layout(&s, scale, point(0.0, 0.0)).collect();
                for glyph in s {
                    if let Some(bounding_box) = glyph.pixel_bounding_box() {
                        glyph.draw(|x, y, v| {
                            let x = x + bounding_box.min.x as u32;
                            let y = y + bounding_box.min.y as u32;
                            let color = Color { r: 255, g: 255, b: 255, a: (v * 255.0) as u8 };
                            pixels.push(Pixel::new(x, y, color));
                        });
                    }
                }
            },
            "PRIVMSG" | "WHISPER" => {
                let v_metrics = font.v_metrics(scale);
                let user = format!("{}:", self.user);
                let user: Vec<_> = font.layout(&user, scale, point(0.0, 0.0)).collect();
                let edge = user.last().map(|g| g.pixel_bounding_box().unwrap().max.x).unwrap();
                for glyph in user {
                    if let Some(bounding_box) = glyph.pixel_bounding_box() {
                        glyph.draw(|x, y, v| {
                            let x = x + bounding_box.min.x as u32;
                            let y = y + bounding_box.min.y as u32;
                            let color = self.tags.color.clone().into((v * 255.0) as u8);
                            pixels.push(Pixel::new(x, y, color));
                        });
                    }
                }
                let mut wid = edge as f32;
                // TODO detect emotes and substitute font with emote image
                // Since the returned vector is already just the individual pixels,
                // This should be a piece of cake with the imageproc crate, just
                // Resize the emote with anti aliasing, and all should be fine.
                let mut words = self.message.split_whitespace();
                while let Some(word) = words.next() {
                    let mut matched = false;
                    let mut emotes = GLOBALS.clone();
                    for (key, e) in self.tags.emotes.iter() {
                        emotes.push((key.to_owned(), e[0].0.to_owned(), EmoteType::TTV));
                    }
                    if let Ok(ref mut channel) = EMOTES.read() {
                        if let Some(room) = channel.get(&self.tags.room_id) {
                            for e in room {
                                emotes.push((e.0.to_owned(), e.1.to_owned(), e.2));
                            }
                        }
                    }
                    for (emote, code, kind) in emotes {
                        if word == emote {
                            let em = get_emote(s.clone(), Emotes::emote(&code, &emote, kind), v_metrics.ascent as u32);
                            for x in 0..em.width() as usize {
                                for y in 0..em.height() as usize {
                                    let p = em.get_pixel(x as u32, y as u32);
                                    let c = Color { r: p[0], g: p[1], b: p[2], a: p[3] };
                                    pixels.push(Pixel::new(wid as u32 + x as u32 + 5, (lines - 2) as u32 * v_metrics.ascent as u32 + y as u32 + 3, c));
                                }
                            }
                            wid += v_metrics.ascent + 2.0;
                            matched = true;
                            break
                        }
                    }
                    if matched {
                        continue
                    }
                    let mut s: Vec<_> = font.layout(word, scale, point(wid + 5.0, (lines - 1) as f32 * v_metrics.ascent)).collect();
                    let edge = s.last().map(|g| g.pixel_bounding_box().unwrap().max.x).unwrap();
                    if edge as u32 >= width - 10 {
                        wid = 10.0 + (edge - s.first().map(|g| g.pixel_bounding_box().unwrap().max.x).unwrap()) as f32;
                        lines += 1;
                        // Yes, redoing the layout is necessary here.
                        s = font.layout(word, scale, point(5.0, (lines - 1) as f32 * v_metrics.ascent)).collect();
                    } else {
                        wid = edge as f32;
                    }
                    for glyph in s {
                        if let Some(bounding_box) = glyph.pixel_bounding_box() {
                            glyph.draw(|x, y, v| {
                                let x = x + bounding_box.min.x as u32;
                                let y = y + bounding_box.min.y as u32;
                                let color = Color { r: 255, g: 255, b: 255, a: (v * 255.0) as u8 };
                                pixels.push(Pixel::new(x, (lines - 2).min(0) as u32 * v_metrics.ascent as u32 + y, color));
                            });
                        }
                    }
                }
            },
            // Various system messages
            "USERNOTICE" | "NOTICE" => {
                match self.tags.msg_id.as_ref() {
                    "sub" | "resub" | "extendsub" => {
                        let v_metrics = font.v_metrics(scale);
                        let system: Vec<_> = font.layout(&self.tags.system_msg, scale, point(0.0, 0.0)).collect();
                        for glyph in system {
                            if let Some(bounding_box) = glyph.pixel_bounding_box() {
                                glyph.draw(|x, y, v| {
                                    let x = x + bounding_box.min.x as u32;
                                    let y = y + bounding_box.min.y as u32;
                                    let color = Color { r: 124, g: 124, b: 128, a: (v * 255.0) as u8 };
                                    pixels.push(Pixel::new(x, y, color));
                                });
                            }
                        }
                        if self.message != "" {
                            let mut user = Vec::new();
                            let mut dummy = self.clone();
                            dummy.command = String::from("PRIVMSG");
                            dummy.user = self.tags.display_name.to_owned();
                            lines += dummy.build_glyphs(s, &mut user, font, scale, width);
                            for pixel in user {
                                pixels.push(Pixel::new(pixel.x, v_metrics.ascent as u32 + pixel.y, pixel.c));
                            }
                        }
                    },
                    "host_on" | "host_off" | "host_target_went_offline" | "emote_only_on" |
                    "emote_only_off" | "followers_on" | "followers_off"=> {
                        let system: Vec<_> = font.layout(&self.message, scale, point(0.0, 0.0)).collect();
                        for glyph in system {
                            if let Some(bounding_box) = glyph.pixel_bounding_box() {
                                glyph.draw(|x, y, v| {
                                    let x = x + bounding_box.min.x as u32;
                                    let y = y + bounding_box.min.y as u32;
                                    let color = Color { r: 124, g: 124, b: 128, a: (v * 255.0) as u8 };
                                    pixels.push(Pixel::new(x, y, color));
                                });
                            }
                        }
                    },
                    "subgift" | "submysterygift" | "standardpayforward" | "communitypayforward" | 
                    "giftpaidupgrade" | "anongiftpaidupgrade" | "primepaidupgrade" | "raid" => {
                        let system: Vec<_> = font.layout(&self.tags.system_msg, scale, point(0.0, 0.0)).collect();
                        for glyph in system {
                            if let Some(bounding_box) = glyph.pixel_bounding_box() {
                                glyph.draw(|x, y, v| {
                                    let x = x + bounding_box.min.x as u32;
                                    let y = y + bounding_box.min.y as u32;
                                    let color = Color { r: 124, g: 124, b: 128, a: (v * 255.0) as u8 };
                                    pixels.push(Pixel::new(x, y, color));
                                });
                            }
                        }
                    },
                    "bitsbadgetier" => {
                        // TODO add support for badges
                    },
                    _ => println!("{:?}", self.raw),
                }
            },
            _ => {
                println!("{:?}", self.raw);
                let s: Vec<_> = font.layout(&self.raw, scale, point(0.0, 0.0)).collect();
                for glyph in s {
                    if let Some(bounding_box) = glyph.pixel_bounding_box() {
                        glyph.draw(|x, y, v| {
                            let x = x + bounding_box.min.x as u32;
                            let y = y + bounding_box.min.y as u32;
                            let color = Color { r: 255, g: 255, b: 255, a: (v * 255.0) as u8 };
                            pixels.push(Pixel::new(x, y, color));
                        });
                    }
                }
            },
        }
        lines
    }
}

#[derive(Clone, Default)]
struct Tags {
    badges: HashMap<String, String>,
    badge_info: HashMap<String, String>,
    emotes: HashMap<String, Vec<(String, usize, usize)>>,
    display_name: String,
    color: Hex,
    flags: String,
    system_msg: String,
    msg_id: String,
    id: String,
    r#mod: bool,
    room_id: String,
    subscriber: bool,
    turbo: bool,
    user_id: String,
    user_type: String,
    gift_recipient: String,
}

impl Tags {
    fn set(&mut self, key: &str, val: &str) {
        match key {
            "color" => self.color = Hex {code: val.to_owned() },
            "display-name" => self.display_name = val.to_owned(),
            "flags" => self.flags = val.to_owned(),
            "msg-id" => self.msg_id = val.to_owned(),
            "system-msg" => self.system_msg = val.replace("\\s", " ").to_owned(),
            "id" => self.id = val.to_owned(),
            "mod" => self.r#mod = match val { "1" => true, _ => false },
            "room-id" => self.room_id = val.to_owned(),
            "subscriber" => self.subscriber = match val { "1" => true, _ => false },
            "turbo" => self.turbo = match val { "1" => true, _ => false },
            "user-id" => self.user_id = val.to_owned(),
            "user-type" => self.user_type = val.to_owned(),
            "msg-param-recipient-user-name" => self.gift_recipient = val.to_owned(),
            _ => {},
        }
    }
}

#[derive(Clone)]
struct Pixel {
    x: u32,
    y: u32,
    c: Color,
}

impl Pixel {
    fn new(x: u32, y: u32, c: Color) -> Self {
        Self {
            x: x,
            y: y,
            c: c,
        }
    }
}

#[derive(Clone, Default)]
struct Hex {
    code: String,
}

impl Hex {
    /// Because this converts from hex codes with only rgb, you have to define the alpha
    fn into(self, a: u8) -> Color {
        let hex = &self.code.replace("#", "");
        let mut rgb = [255u8; 3];
        match hex::decode_to_slice(hex, &mut rgb) {
            Ok(_) => {
                let mut r = 0;
                let mut g = 0;
                let mut b = 0;
                if rgb[0] < 30 && rgb[1] < 30 && rgb[2] < 30 {
                    r += 78;
                    g += 78;
                    b += 78;
                }
                if rgb[0] < 30 && rgb[1] < 30 && rgb[2] > 200 {
                    r += 100;
                    g += 100;
                }
                Color {
                    r: rgb[0] + r,
                    g: rgb[1] + g,
                    b: rgb[2] + b,
                    a: a,
                }
            },
            Err(_) => Color { r: 255, g: 255, b: 255, a: a },
        }
    }
}

#[derive(Clone, Debug)]
struct Emotes {
    kind: EmoteType,
    code: String,
    id: String,
}

impl Emotes {
    fn emote(id: &str, code: &str, kind: EmoteType) -> Self {
        Self {
            kind: kind,
            code: code.to_owned(),
            id: id.to_owned(),
        }
    }

    fn room(id: &str) -> Self {
        Self {
            kind: EmoteType::ROOM,
            code: id.to_owned(),
            id: id.to_owned(),
        }
    }
}

#[derive(Copy, Clone, Debug)]
enum EmoteType {
    TTV,
    BTTV,
    FFZ,
    ROOM,
}

#[derive(Clone, Deserialize, Serialize)]
struct BttvChannel {
    id: String,
    bots: Vec<String>,
    #[serde(rename = "channelEmotes")]
    channel_emotes: Vec<BttvEmote>,
    #[serde(rename = "sharedEmotes")]
    shared_emotes: Vec<BttvShared>,
}

impl BttvChannel {
    fn emotes(&self) -> Vec<Emotes> {
        let mut v = Vec::new();
        for emote in &self.channel_emotes {
            v.push(Emotes::emote(&emote.id, &emote.code, EmoteType::BTTV));
        }
        for emote in &self.shared_emotes {
            v.push(Emotes::emote(&emote.id, &emote.code, EmoteType::BTTV));
        }
        v
    }
}

#[derive(Clone, Deserialize, Serialize)]
struct BttvEmote {
    id: String,
    code: String,
    #[serde(rename = "imageType")]
    _image_type: String,
    #[serde(rename = "userId")]
    _user_id: String,
}

#[derive(Clone, Deserialize, Serialize)]
struct BttvShared {
    id: String,
    code: String,
    #[serde(rename = "imageType", skip_deserializing)]
    _image_type: String,
    #[serde(rename = "user", skip_deserializing)]
    user: String,
}

#[derive(Clone, Deserialize, Serialize)]
struct FfzEmote {
    id: u32,
    #[serde(skip_deserializing)]
    user: String,
    code: String,
    #[serde(skip_deserializing)]
    images: String,
    #[serde(rename = "imageType", skip_deserializing)]
    _image_type: String,
}

#[derive(Debug)]
pub enum Error<T> {
    IO(std::io::Error),
    STR(String),
    ANY(T),
}

type Result<T> = std::result::Result<T, Error<String>>;

impl<T> From<String> for Error<T> {
    fn from(r: String) -> Error<T> {
        Error::STR(r)
    }
}

impl<T> From<std::io::Error> for Error<T> {
    fn from(e: std::io::Error) -> Error<T> {
        Error::IO(e)
    }
}
