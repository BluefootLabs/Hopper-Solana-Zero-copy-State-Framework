//! Interactive terminal UI for the Hopper Manager.
//!
//! Provides a menu-driven interface for exploring program manifests,
//! inspecting layouts, decoding accounts, and running diagnostics —
//! all within a single terminal session.

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{self, ClearType},
};
use std::io::{self, Write};

use hopper_schema::{
    LayoutFingerprint,
    ProgramManifest, decode_account_fields, decode_header,
};

// ---------------------------------------------------------------------------
// View enum — what the user is looking at
// ---------------------------------------------------------------------------

#[derive(Clone)]
enum View {
    MainMenu,
    Summary,
    Layouts,
    LayoutDetail(usize),
    Instructions,
    InstructionDetail(usize),
    Policies,
    PolicyDetail(usize),
    Events,
    EventDetail(usize),
    DecodePrompt,
    DecodeResult(String),
    Help,
}

// ---------------------------------------------------------------------------
// Interactive session state
// ---------------------------------------------------------------------------

pub struct Session<'a> {
    prog: &'a ProgramManifest,
    view: View,
    cursor: usize,
    history: Vec<(View, usize)>,
    status: String,
}

impl<'a> Session<'a> {
    pub fn new(prog: &'a ProgramManifest) -> Self {
        Self {
            prog,
            view: View::MainMenu,
            cursor: 0,
            history: Vec::new(),
            status: String::new(),
        }
    }

    /// Run the interactive session. Blocks until the user quits.
    pub fn run(&mut self) -> io::Result<()> {
        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, terminal::EnterAlternateScreen, cursor::Hide)?;

        loop {
            self.draw(&mut stdout)?;

            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    break;
                }
                if !self.handle_key(key) {
                    break;
                }
            }
        }

        execute!(stdout, cursor::Show, terminal::LeaveAlternateScreen)?;
        terminal::disable_raw_mode()?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Drawing
    // -----------------------------------------------------------------------

    fn draw(&self, w: &mut impl Write) -> io::Result<()> {
        execute!(w, cursor::MoveTo(0, 0), terminal::Clear(ClearType::All))?;

        let (cols, rows) = terminal::size()?;
        let width = cols as usize;

        // Title bar
        let title = format!(" HOPPER MANAGER — {} {} ", self.prog.name, self.prog.version);
        let pad = width.saturating_sub(title.len());
        write!(w, "\x1b[7m{}{}\x1b[0m\r\n", title, " ".repeat(pad))?;
        write!(w, "\r\n")?;

        match &self.view {
            View::MainMenu => self.draw_main_menu(w, width)?,
            View::Summary => self.draw_summary(w)?,
            View::Layouts => self.draw_layouts(w)?,
            View::LayoutDetail(idx) => self.draw_layout_detail(w, *idx)?,
            View::Instructions => self.draw_instructions(w)?,
            View::InstructionDetail(idx) => self.draw_instruction_detail(w, *idx)?,
            View::Policies => self.draw_policies(w)?,
            View::PolicyDetail(idx) => self.draw_policy_detail(w, *idx)?,
            View::Events => self.draw_events(w)?,
            View::EventDetail(idx) => self.draw_event_detail(w, *idx)?,
            View::DecodePrompt => self.draw_decode_prompt(w)?,
            View::DecodeResult(ref text) => self.draw_decode_result(w, text)?,
            View::Help => self.draw_help(w)?,
        }

        // Status bar at bottom
        let status_row = rows.saturating_sub(1);
        execute!(w, cursor::MoveTo(0, status_row))?;
        let status_text = if self.status.is_empty() {
            " [↑↓] Navigate  [Enter] Select  [Esc/Backspace] Back  [q] Quit  [?] Help".to_string()
        } else {
            format!(" {}", self.status)
        };
        let spad = width.saturating_sub(status_text.len());
        write!(w, "\x1b[7m{}{}\x1b[0m", status_text, " ".repeat(spad))?;

        w.flush()?;
        Ok(())
    }

    fn draw_main_menu(&self, w: &mut impl Write, _width: usize) -> io::Result<()> {
        let items = self.main_menu_items();
        for (i, (label, count)) in items.iter().enumerate() {
            let marker = if i == self.cursor { "▸ " } else { "  " };
            let highlight = if i == self.cursor { "\x1b[1;36m" } else { "" };
            write!(w, "{}{}{:<30}\x1b[0m{}\r\n",
                marker, highlight, label, count)?;
        }
        Ok(())
    }

    fn main_menu_items(&self) -> Vec<(&str, String)> {
        vec![
            ("Program Summary", String::new()),
            ("Layouts", format!("({})", self.prog.layouts.len())),
            ("Instructions", format!("({})", self.prog.instructions.len())),
            ("Policies", format!("({})", self.prog.policies.len())),
            ("Events", format!("({})", self.prog.events.len())),
            ("Decode Account (hex)", String::new()),
            ("Help", String::new()),
        ]
    }

    fn draw_summary(&self, w: &mut impl Write) -> io::Result<()> {
        let p = self.prog;
        write!(w, "  \x1b[1mProgram:\x1b[0m  {}\r\n", p.name)?;
        write!(w, "  \x1b[1mVersion:\x1b[0m  {}\r\n", p.version)?;
        write!(w, "  \x1b[1mDesc:\x1b[0m     {}\r\n", p.description)?;
        write!(w, "\r\n")?;
        write!(w, "  Layouts:      {}\r\n", p.layouts.len())?;
        write!(w, "  Instructions: {}\r\n", p.instructions.len())?;
        write!(w, "  Policies:     {}\r\n", p.policies.len())?;
        write!(w, "  Events:       {}\r\n", p.events.len())?;
        write!(w, "\r\n")?;

        // Layout overview
        if !p.layouts.is_empty() {
            write!(w, "  \x1b[1;33mLayouts:\x1b[0m\r\n")?;
            for l in p.layouts.iter() {
                let fp = LayoutFingerprint::from_manifest(l);
                write!(w, "    {} v{} — {} bytes, {} fields  [wire:{} sem:{}]\r\n",
                    l.name, l.version, l.total_size, l.field_count,
                    hex_short(&fp.wire_hash), hex_short(&fp.semantic_hash))?;
            }
        }

        write!(w, "\r\n")?;
        // Instruction overview
        if !p.instructions.is_empty() {
            write!(w, "  \x1b[1;33mInstructions:\x1b[0m\r\n")?;
            for ix in p.instructions.iter() {
                write!(w, "    [{}] {} — {} args, {} accounts\r\n",
                    ix.tag, ix.name, ix.args.len(), ix.accounts.len())?;
            }
        }

        Ok(())
    }

    fn draw_layouts(&self, w: &mut impl Write) -> io::Result<()> {
        if self.prog.layouts.is_empty() {
            write!(w, "  (no layouts defined)\r\n")?;
            return Ok(());
        }
        for (i, l) in self.prog.layouts.iter().enumerate() {
            let marker = if i == self.cursor { "▸ " } else { "  " };
            let highlight = if i == self.cursor { "\x1b[1;36m" } else { "" };
            write!(w, "{}{}{} v{}  \x1b[0m— {} bytes, {} fields\r\n",
                marker, highlight, l.name, l.version, l.total_size, l.field_count)?;
        }
        Ok(())
    }

    fn draw_layout_detail(&self, w: &mut impl Write, idx: usize) -> io::Result<()> {
        let l = &self.prog.layouts[idx];
        let fp = LayoutFingerprint::from_manifest(l);

        write!(w, "  \x1b[1m{} v{}\x1b[0m\r\n", l.name, l.version)?;
        write!(w, "  Disc: {}  Size: {} bytes  Fields: {}\r\n",
            l.disc, l.total_size, l.field_count)?;
        write!(w, "  Wire hash:     {}\r\n", hex_encode(&fp.wire_hash))?;
        write!(w, "  Semantic hash: {}\r\n", hex_encode(&fp.semantic_hash))?;
        write!(w, "\r\n")?;
        write!(w, "  \x1b[1;33mFields:\x1b[0m\r\n")?;
        write!(w, "  {:<4} {:<20} {:<14} {:<6} {:<6} {}\r\n",
            "#", "Name", "Type", "Offset", "Size", "Intent")?;
        write!(w, "  {}\r\n", "─".repeat(70))?;
        for (i, f) in l.fields.iter().enumerate() {
            let intent_label = f.intent.name();
            write!(w, "  {:<4} {:<20} {:<14} {:<6} {:<6} {}\r\n",
                i, f.name, f.canonical_type, f.offset, f.size, intent_label)?;
        }

        // Segments - check layout_metadata for segment info
        let seg_meta = self.prog.layout_metadata.iter()
            .find(|m| m.name == l.name);
        if let Some(meta) = seg_meta {
            if !meta.segment_roles.is_empty() {
                write!(w, "\r\n  \x1b[1;33mSegment Roles:\x1b[0m\r\n")?;
                for (i, role) in meta.segment_roles.iter().enumerate() {
                    write!(w, "    [{}] {}\r\n", i, role)?;
                }
            }
        }
        Ok(())
    }

    fn draw_instructions(&self, w: &mut impl Write) -> io::Result<()> {
        if self.prog.instructions.is_empty() {
            write!(w, "  (no instructions defined)\r\n")?;
            return Ok(());
        }
        for (i, ix) in self.prog.instructions.iter().enumerate() {
            let marker = if i == self.cursor { "▸ " } else { "  " };
            let highlight = if i == self.cursor { "\x1b[1;36m" } else { "" };
            write!(w, "{}{}[{}] {}\x1b[0m — {} args, {} accounts\r\n",
                marker, highlight, ix.tag, ix.name, ix.args.len(), ix.accounts.len())?;
        }
        Ok(())
    }

    fn draw_instruction_detail(&self, w: &mut impl Write, idx: usize) -> io::Result<()> {
        let ix = &self.prog.instructions[idx];
        write!(w, "  \x1b[1m[{}] {}\x1b[0m\r\n", ix.tag, ix.name)?;
        write!(w, "  Policy:      {}\r\n", ix.policy_pack)?;
        write!(w, "  Receipt:     {}\r\n", if ix.receipt_expected { "yes" } else { "no" })?;
        write!(w, "\r\n")?;

        if !ix.args.is_empty() {
            write!(w, "  \x1b[1;33mArguments:\x1b[0m\r\n")?;
            for arg in ix.args.iter() {
                write!(w, "    {}: {} ({} bytes)\r\n",
                    arg.name, arg.canonical_type, arg.size)?;
            }
        }

        if !ix.accounts.is_empty() {
            write!(w, "\r\n  \x1b[1;33mAccounts:\x1b[0m\r\n")?;
            for acc in ix.accounts.iter() {
                let mut flags = Vec::new();
                if acc.signer { flags.push("signer"); }
                if acc.writable { flags.push("writable"); }
                let flag_str = if flags.is_empty() { String::new() }
                    else { format!(" [{}]", flags.join(", ")) };
                write!(w, "    {}{}\r\n", acc.name, flag_str)?;
            }
        }
        Ok(())
    }

    fn draw_policies(&self, w: &mut impl Write) -> io::Result<()> {
        if self.prog.policies.is_empty() {
            write!(w, "  (no policies defined)\r\n")?;
            return Ok(());
        }
        for (i, pol) in self.prog.policies.iter().enumerate() {
            let marker = if i == self.cursor { "▸ " } else { "  " };
            let highlight = if i == self.cursor { "\x1b[1;36m" } else { "" };
            write!(w, "{}{}{}\x1b[0m — {} caps, {} reqs\r\n",
                marker, highlight, pol.name,
                pol.capabilities.len(), pol.requirements.len())?;
        }
        Ok(())
    }

    fn draw_policy_detail(&self, w: &mut impl Write, idx: usize) -> io::Result<()> {
        let pol = &self.prog.policies[idx];
        write!(w, "  \x1b[1m{}\x1b[0m\r\n", pol.name)?;
        write!(w, "  Receipt profile: {}\r\n", pol.receipt_profile)?;
        write!(w, "\r\n")?;

        if !pol.invariants.is_empty() {
            write!(w, "  \x1b[1;33mInvariants:\x1b[0m\r\n")?;
            for inv in pol.invariants.iter() {
                write!(w, "    {}\r\n", inv)?;
            }
        }

        if !pol.capabilities.is_empty() {
            write!(w, "\r\n  \x1b[1;33mCapabilities:\x1b[0m\r\n")?;
            for cap in pol.capabilities.iter() {
                write!(w, "    {}\r\n", cap)?;
            }
        }

        if !pol.requirements.is_empty() {
            write!(w, "\r\n  \x1b[1;33mRequirements:\x1b[0m\r\n")?;
            for req in pol.requirements.iter() {
                write!(w, "    {}\r\n", req)?;
            }
        }
        Ok(())
    }

    fn draw_events(&self, w: &mut impl Write) -> io::Result<()> {
        if self.prog.events.is_empty() {
            write!(w, "  (no events defined)\r\n")?;
            return Ok(());
        }
        for (i, ev) in self.prog.events.iter().enumerate() {
            let marker = if i == self.cursor { "▸ " } else { "  " };
            let highlight = if i == self.cursor { "\x1b[1;36m" } else { "" };
            write!(w, "{}{}[{}] {}\x1b[0m — {} fields\r\n",
                marker, highlight, ev.tag, ev.name, ev.fields.len())?;
        }
        Ok(())
    }

    fn draw_event_detail(&self, w: &mut impl Write, idx: usize) -> io::Result<()> {
        let ev = &self.prog.events[idx];
        write!(w, "  \x1b[1m[{}] {}\x1b[0m\r\n", ev.tag, ev.name)?;
        write!(w, "\r\n")?;

        if !ev.fields.is_empty() {
            write!(w, "  \x1b[1;33mFields:\x1b[0m\r\n")?;
            for fd in ev.fields.iter() {
                write!(w, "    {}: {} ({} bytes)\r\n",
                    fd.name, fd.canonical_type, fd.size)?;
            }
        }
        Ok(())
    }

    fn draw_decode_prompt(&self, w: &mut impl Write) -> io::Result<()> {
        write!(w, "  \x1b[1mDecode Account from Hex\x1b[0m\r\n")?;
        write!(w, "\r\n")?;
        write!(w, "  Paste hex-encoded account data and press Enter.\r\n")?;
        write!(w, "  (Must be at least 16 bytes for a valid Hopper header)\r\n")?;
        write!(w, "\r\n")?;
        write!(w, "  > ")?;
        Ok(())
    }

    fn draw_decode_result(&self, w: &mut impl Write, text: &str) -> io::Result<()> {
        for line in text.lines() {
            write!(w, "  {}\r\n", line)?;
        }
        Ok(())
    }

    fn draw_help(&self, w: &mut impl Write) -> io::Result<()> {
        write!(w, "  \x1b[1mHopper Interactive Manager — Help\x1b[0m\r\n")?;
        write!(w, "\r\n")?;
        write!(w, "  \x1b[1;33mNavigation:\x1b[0m\r\n")?;
        write!(w, "    ↑/k       Move up\r\n")?;
        write!(w, "    ↓/j       Move down\r\n")?;
        write!(w, "    Enter     Select / drill into\r\n")?;
        write!(w, "    Esc/Bksp  Go back\r\n")?;
        write!(w, "    q         Quit\r\n")?;
        write!(w, "    ?         This help screen\r\n")?;
        write!(w, "\r\n")?;
        write!(w, "  \x1b[1;33mViews:\x1b[0m\r\n")?;
        write!(w, "    Summary        Program overview with all stats\r\n")?;
        write!(w, "    Layouts        Browse all account layouts and fields\r\n")?;
        write!(w, "    Instructions   Browse instructions, args, accounts\r\n")?;
        write!(w, "    Policies       Browse policy packs and capabilities\r\n")?;
        write!(w, "    Events         Browse event schemas\r\n")?;
        write!(w, "    Decode         Paste hex data to decode an account\r\n")?;
        write!(w, "\r\n")?;
        write!(w, "  Each layout detail view shows fields, types, offsets,\r\n")?;
        write!(w, "  intents, segments, and fingerprint hashes.\r\n")?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Input handling
    // -----------------------------------------------------------------------

    /// Handle a key event. Returns false if the session should exit.
    fn handle_key(&mut self, key: KeyEvent) -> bool {
        self.status.clear();

        match key.code {
            KeyCode::Char('q') => {
                if matches!(self.view, View::DecodePrompt) {
                    self.go_back();
                    return true;
                }
                return false;
            }
            KeyCode::Char('?') => {
                if !matches!(self.view, View::Help) {
                    self.push_view(View::Help);
                }
            }
            KeyCode::Esc | KeyCode::Backspace => {
                self.go_back();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let max = self.item_count();
                if max > 0 && self.cursor < max - 1 {
                    self.cursor += 1;
                }
            }
            KeyCode::Enter => {
                self.select_current();
            }
            _ => {}
        }
        true
    }

    fn item_count(&self) -> usize {
        match &self.view {
            View::MainMenu => self.main_menu_items().len(),
            View::Layouts => self.prog.layouts.len(),
            View::Instructions => self.prog.instructions.len(),
            View::Policies => self.prog.policies.len(),
            View::Events => self.prog.events.len(),
            _ => 0,
        }
    }

    fn select_current(&mut self) {
        match &self.view {
            View::MainMenu => {
                let next = match self.cursor {
                    0 => View::Summary,
                    1 => View::Layouts,
                    2 => View::Instructions,
                    3 => View::Policies,
                    4 => View::Events,
                    5 => View::DecodePrompt,
                    6 => View::Help,
                    _ => return,
                };
                self.push_view(next);
            }
            View::Layouts => {
                if self.cursor < self.prog.layouts.len() {
                    self.push_view(View::LayoutDetail(self.cursor));
                }
            }
            View::Instructions => {
                if self.cursor < self.prog.instructions.len() {
                    self.push_view(View::InstructionDetail(self.cursor));
                }
            }
            View::Policies => {
                if self.cursor < self.prog.policies.len() {
                    self.push_view(View::PolicyDetail(self.cursor));
                }
            }
            View::Events => {
                if self.cursor < self.prog.events.len() {
                    self.push_view(View::EventDetail(self.cursor));
                }
            }
            View::DecodePrompt => {
                self.run_decode_input();
            }
            _ => {}
        }
    }

    fn push_view(&mut self, new: View) {
        self.history.push((self.view.clone(), self.cursor));
        self.view = new;
        self.cursor = 0;
    }

    fn go_back(&mut self) {
        if let Some((prev_view, prev_cursor)) = self.history.pop() {
            self.view = prev_view;
            self.cursor = prev_cursor;
        }
    }

    fn run_decode_input(&mut self) {
        // Switch out of raw mode temporarily to read a line
        let _ = terminal::disable_raw_mode();
        let mut stdout = io::stdout();
        // Show cursor for input
        let _ = execute!(stdout, cursor::Show);

        let mut input = String::new();
        let _ = io::stdin().read_line(&mut input);
        let input = input.trim();

        let _ = execute!(stdout, cursor::Hide);
        let _ = terminal::enable_raw_mode();

        if input.is_empty() {
            self.status = "No input provided".to_string();
            return;
        }

        let result = self.decode_hex_account(input);
        self.history.push((self.view.clone(), self.cursor));
        self.view = View::DecodeResult(result);
        self.cursor = 0;
    }

    fn decode_hex_account(&self, hex: &str) -> String {
        let data = match hex_decode_bytes(hex) {
            Ok(d) => d,
            Err(e) => return format!("Hex decode error: {}", e),
        };

        if data.len() < 16 {
            return format!("Data too short ({} bytes, need 16 for header)", data.len());
        }

        let header = match decode_header(&data) {
            Some(h) => h,
            None => return "Failed to decode header".to_string(),
        };

        let mut out = String::new();
        out.push_str(&format!("\x1b[1mAccount Header\x1b[0m\n"));
        out.push_str(&format!("  Disc:      {}\n", header.disc));
        out.push_str(&format!("  Version:   {}\n", header.version));
        out.push_str(&format!("  Layout ID: {}\n", hex_encode(&header.layout_id)));
        out.push_str(&format!("  Data size: {} bytes\n", data.len()));
        out.push_str("\n");

        // Try to identify layout
        match self.prog.identify_from_data(&data) {
            Some(layout) => {
                out.push_str(&format!("\x1b[1;32mIdentified: {} v{}\x1b[0m\n",
                    layout.name, layout.version));
                out.push_str(&format!("  Expected size: {} bytes\n", layout.total_size));
                out.push_str(&format!("  Fields:        {}\n", layout.field_count));
                out.push_str("\n");

                // Decode fields
                let (_count, fields) = decode_account_fields::<64>(&data, layout);
                out.push_str("\x1b[1;33mField Values:\x1b[0m\n");
                for (i, f) in layout.fields.iter().enumerate() {
                    let val = fields[i].as_ref()
                        .map(|fv| hex_encode(fv.raw))
                        .unwrap_or_else(|| "(unavailable)".to_string());
                    out.push_str(&format!("  {:<20} {} = {}\n",
                        f.name, f.canonical_type, val));
                }
            }
            None => {
                out.push_str("\x1b[1;31mNo matching layout found in manifest\x1b[0m\n");
                out.push_str("\nKnown layouts:\n");
                for l in self.prog.layouts.iter() {
                    out.push_str(&format!("  {} v{} (id={})\n",
                        l.name, l.version, hex_encode(&l.layout_id)));
                }
            }
        }

        out
    }
}

// ---------------------------------------------------------------------------
// Hex utility functions (local to this module)
// ---------------------------------------------------------------------------

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn hex_short(bytes: &[u8]) -> String {
    if bytes.len() <= 4 {
        hex_encode(bytes)
    } else {
        format!("{}..{}", hex_encode(&bytes[..2]), hex_encode(&bytes[bytes.len()-2..]))
    }
}

fn hex_decode_bytes(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return Err("Hex string must have even length".to_string());
    }
    let mut bytes = Vec::with_capacity(s.len() / 2);
    let chars: Vec<u8> = s.bytes().collect();
    for pair in chars.chunks(2) {
        let high = hex_nibble(pair[0])?;
        let low = hex_nibble(pair[1])?;
        bytes.push((high << 4) | low);
    }
    Ok(bytes)
}

fn hex_nibble(b: u8) -> Result<u8, String> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(format!("Invalid hex character: {}", b as char)),
    }
}
