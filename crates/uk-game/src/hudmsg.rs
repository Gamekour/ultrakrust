//! HudMessageReceiver (the player Canvas' MessageHud), HudMessage (the hint triggers) and
//! ScrollingText.ShowText, the typewriter the receiver's text is written with.

use crate::game::{Act, Game};
use crate::scripts::Script;

/// HudMessageReceiver's state (`State::msg`). It lives outside the UI so headless runs keep it;
/// the TMP text and the Image / text enables are written through when a UI exists.
#[derive(Clone, Debug, Default)]
pub struct MsgReceiver {
    /// Image / TMP_Text / HudOpenEffect script indices (Start)
    pub img: Option<u32>,
    pub text: Option<u32>,
    pub hoe: Option<u32>,
    pub started: bool,
    message: String,
    inputs: Option<Vec<String>>,
    input_pre: bool,
    pub no_sound: bool,
    timer: bool,
    pub full: String,
    /// TMP_Text.text as ShowText leaves it
    pub shown_text: String,
    /// ShowText running: (message chars, current letter, TimeSince start, WaitForSeconds resume time)
    routine: Option<Scroll>,
}

#[derive(Clone, Debug)]
struct Scroll {
    message: Vec<char>,
    letter: usize,
    timer_start: f64,
    resume_at: f64,
}

const SECONDS_BETWEEN_LETTERS: f32 = 0.005;

/// string.Format(format, args) for `{n}` items and `{{` / `}}` escapes (alignment and format
/// strings are not used by the hints). None where .NET throws a FormatException.
pub fn net_format(format: &str, args: &[String]) -> Option<String> {
    let c: Vec<char> = format.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '{' if c.get(i + 1) == Some(&'{') => {
                out.push('{');
                i += 2;
            }
            '}' if c.get(i + 1) == Some(&'}') => {
                out.push('}');
                i += 2;
            }
            '}' => return None,
            '{' => {
                let end = c[i..].iter().position(|&x| x == '}')? + i;
                let item: String = c[i + 1..end].iter().collect();
                let idx = item.split([',', ':']).next()?.trim();
                let n: usize = idx.parse().ok()?;
                out += args.get(n)?;
                i = end + 1;
            }
            x => {
                out.push(x);
                i += 1;
            }
        }
    }
    Some(out)
}

impl Game {
    /// HudMessageReceiver.Start: Image and HudOpenEffect on its object, the first TMP_Text in children.
    pub(crate) fn msg_receiver_start(&mut self, sc: u32) {
        let node = self.def.scripts[sc as usize].node;
        let img = self.def.scripts_on(node).find(|(_, s)| s.class == "Image").map(|(i, _)| i);
        let hoe = self.def.scripts_on(node).find(|(_, s)| s.class == "HudOpenEffect").map(|(i, _)| i);
        let text = self.child_script_where(node, |c| c == "TextMeshProUGUI" || c == "TextMeshPro");
        let m = &mut self.s.msg;
        m.img = img;
        m.text = text;
        m.hoe = hoe;
        m.started = true;
    }

    /// GetComponentInChildren by class name (self first, depth-first, active objects only).
    fn child_script_where(&self, node: u32, pred: impl Fn(&str) -> bool) -> Option<u32> {
        let mut stack = vec![node];
        while let Some(m) = stack.pop() {
            if !self.s.active[m as usize] && m != node {
                continue;
            }
            if let Some((i, _)) = self.def.scripts_on(m).find(|(_, s)| pred(&s.class)) {
                return Some(i);
            }
            stack.extend(self.def.nodes[m as usize].children.iter().rev().copied());
        }
        None
    }

    /// MonoSingleton<HudMessageReceiver>.Instance (set in Awake).
    fn msg_receiver(&self) -> Option<u32> {
        self.msg_receiver.filter(|&r| self.s.script_awake[r as usize])
    }

    /// HudMessageReceiver.SendHudMessage
    #[allow(clippy::too_many_arguments)]
    pub fn send_hud_message_full(&mut self, newmessage: &str, newinput: &str, newmessage2: &str, delay: f32, silent: bool, input_pre: bool, automatic_timer: bool) {
        let Some(r) = self.msg_receiver() else { return };
        let m = &mut self.s.msg;
        m.message = if newinput.is_empty() { newmessage.to_string() } else { format!("{newmessage}{{0}}{newmessage2}") };
        m.inputs = if newinput.is_empty() { None } else { Some(vec![newinput.to_string()]) };
        m.no_sound = silent;
        m.timer = automatic_timer;
        m.input_pre = input_pre;
        self.invoke(r, Act::ShowHudMessage, delay);
    }

    /// HudMessageReceiver.SendHudMessage2
    pub fn send_hud_message2(&mut self, format: &str, inputs: Option<Vec<String>>, delay: f32, silent: bool, input_pre: bool, automatic_timer: bool) {
        let Some(r) = self.msg_receiver() else { return };
        let m = &mut self.s.msg;
        m.message = format.to_string();
        m.inputs = inputs;
        m.no_sound = silent;
        m.timer = automatic_timer;
        m.input_pre = input_pre;
        self.invoke(r, Act::ShowHudMessage, delay);
    }

    /// SendHudMessage(text) with the defaults (automatic 5 s timer).
    pub fn send_hud_message(&mut self, text: &str) {
        self.send_hud_message_full(text, "", "", 0.0, false, false, true);
    }

    /// HudMessageReceiver.ShowHudMessage
    pub(crate) fn show_hud_message(&mut self, r: u32) {
        let m = &self.s.msg;
        let full = match &m.inputs {
            None => Some(m.message.clone()),
            Some(i) if i.is_empty() => Some(m.message.clone()),
            // the legacy InputManager.Inputs KeyCode names are not ported: the input name itself
            Some(i) => net_format(&m.message, i),
        };
        // string.Format threw: the rest of ShowHudMessage does not run
        let Some(full) = full else { return };
        let full = full.replace('$', "\n");
        self.s.msg.full = full.clone();
        self.msg_set_text(String::new());
        // hoe.Force(): HudOpenEffect.Initialize (values already taken in Awake)
        // aud.Play: no UI audio yet
        self.s.msg.routine = None;
        // PrepText
        self.msg_set_enabled(true);
        self.s.msg.routine = Some(Scroll { message: full.chars().collect(), letter: 0, timer_start: self.s.time, resume_at: self.s.time });
        self.msg_scroll_step();
        self.cancel_invoke(r, Act::HudMessageDone);
        if self.s.msg.timer {
            self.invoke(r, Act::HudMessageDone, 5.0);
        }
    }

    /// The ShowText coroutine's loop body until its next yield (WaitForSeconds(secondsBetweenLetters)).
    /// skipLineBreaks and writingCursor are on, fillMissingText off.
    fn msg_scroll_step(&mut self) {
        let now = self.s.time;
        let Some(sc) = &mut self.s.msg.routine else { return };
        if now < sc.resume_at {
            return;
        }
        let msg = &sc.message;
        let len = msg.len();
        let mut changed = None;
        if sc.letter < len {
            while (now - sc.timer_start) as f32 >= SECONDS_BETWEEN_LETTERS && sc.letter < len {
                sc.timer_start += SECONDS_BETWEEN_LETTERS as f64;
                if msg[sc.letter] == '<' {
                    while sc.letter < len && msg[sc.letter] != '>' {
                        sc.letter += 1;
                    }
                } else if sc.letter < len - 1 {
                    while sc.letter < len - 1 && msg[sc.letter + 1] == '\n' {
                        sc.letter += 1;
                    }
                }
                sc.letter = (sc.letter + 1).min(len);
                let mut t: String = msg[..sc.letter].iter().collect();
                if sc.letter < len {
                    t.push('█');
                }
                changed = Some(t);
            }
        }
        if sc.letter >= len {
            self.s.msg.routine = None;
        } else {
            sc.resume_at = now + SECONDS_BETWEEN_LETTERS as f64;
        }
        if let Some(t) = changed {
            self.msg_set_text(t);
        }
    }

    /// The receiver's per-frame coroutine resume.
    pub(crate) fn msg_update(&mut self) {
        if self.s.msg.routine.is_some() {
            self.msg_scroll_step();
        }
    }

    fn msg_set_text(&mut self, t: String) {
        self.s.msg.shown_text = t;
        self.msg_sync_ui();
    }

    /// TMP text write-through (also after set_ui rebuilt the UI state).
    pub(crate) fn msg_sync_ui(&mut self) {
        if self.s.msg.started {
            let (sc, t) = (self.s.msg.text, self.s.msg.shown_text.clone());
            self.set_tmp_text(sc, &t);
        }
    }

    /// img.enabled / text.enabled
    fn msg_set_enabled(&mut self, on: bool) {
        if let Some(i) = self.s.msg.img {
            self.set_script_enabled(i, on);
        }
        if let Some(t) = self.s.msg.text {
            self.set_script_enabled(t, on);
        }
    }

    /// Is the message box showing (Image enabled)?
    pub fn msg_visible(&self) -> bool {
        self.s.msg.img.is_some_and(|i| self.s.script_enabled[i as usize] && self.s.active[self.def.scripts[i as usize].node as usize])
    }

    /// HudMessageReceiver.Done
    pub(crate) fn msg_done(&mut self) {
        self.msg_set_enabled(false);
    }

    /// HudMessageReceiver.ForceEnable
    fn msg_force_enable(&mut self) {
        if self.msg_receiver().is_some() {
            self.msg_set_enabled(true);
        }
    }

    /// HudMessageReceiver.ClearMessage
    fn msg_clear(&mut self) {
        if let Some(r) = self.msg_receiver() {
            self.cancel_invoke(r, Act::HudMessageDone);
            self.msg_done();
        }
    }

    // ------------------------------------------------------------ HudMessage

    /// HudMessage.PlayerPref (SecMisTut / ShoUseTut aliases)
    fn hud_message_pref(p: &str) -> &str {
        match p {
            "SecMisTut" => "secretMissionPopup",
            "ShoUseTut" => "hideShotgunPopup",
            p => p,
        }
    }

    /// The PlayerPref gate in Start / OnEnable / OnTriggerEnter: an empty pref plays, else the
    /// pref is set (in memory) and the message plays once.
    fn hud_message_gate(&mut self, sc: u32) {
        let Script::HudMessage(h) = &self.s.scripts[sc as usize] else { return };
        let pref = Self::hud_message_pref(&h.player_pref).to_string();
        if pref.is_empty() {
            self.hud_message_play(sc, false);
        } else if !self.prefs.flag(&pref) {
            self.prefs.set_bool(&pref, true);
            self.hud_message_play(sc, false);
        }
    }

    /// HudMessage.Start
    pub(crate) fn hud_message_start(&mut self, sc: u32) {
        let node = self.def.scripts[sc as usize].node;
        if self.def.colliders.iter().any(|c| c.node == node) {
            return;
        }
        if let Script::HudMessage(h) = &mut self.s.scripts[sc as usize] {
            if h.destroyed {
                return;
            }
            h.colliderless = true;
        }
        self.hud_message_gate(sc);
    }

    /// HudMessage.OnEnable
    pub(crate) fn hud_message_enable(&mut self, sc: u32) {
        let Script::HudMessage(h) = &self.s.scripts[sc as usize] else { return };
        if !h.destroyed && h.colliderless && (!h.activated || h.not_one_time) {
            self.hud_message_gate(sc);
        }
    }

    /// HudMessage.OnDisable
    pub(crate) fn hud_message_disable(&mut self, sc: u32) {
        let Script::HudMessage(h) = &self.s.scripts[sc as usize] else { return };
        if !h.destroyed && h.deactive_on_disable && h.activated {
            self.hud_message_done(sc);
        }
    }

    /// HudMessage.Update
    pub(crate) fn hud_message_update(&mut self, sc: u32) {
        let Script::HudMessage(h) = &self.s.scripts[sc as usize] else { return };
        if !h.destroyed && h.activated && h.timed {
            self.msg_force_enable();
        }
    }

    /// HudMessage.OnTriggerEnter / OnTriggerExit (the player)
    pub(crate) fn hud_message_trigger(&mut self, sc: u32, enter: bool) {
        let Script::HudMessage(h) = &self.s.scripts[sc as usize] else { return };
        if h.destroyed || h.dont_on_trigger {
            return;
        }
        if enter {
            if !h.activated || h.not_one_time {
                self.hud_message_gate(sc);
            }
        } else if h.activated && h.deactivate_on_exit {
            self.hud_message_done(sc);
        }
    }

    /// HudMessage.Done
    pub(crate) fn hud_message_done(&mut self, sc: u32) {
        if let Script::HudMessage(h) = &mut self.s.scripts[sc as usize] {
            h.activated = false;
        }
        self.msg_clear();
        self.hud_message_begone(sc);
    }

    /// HudMessage.Begone: Destroy(this) unless notOneTime (its Invokes go with it).
    pub(crate) fn hud_message_begone(&mut self, sc: u32) {
        if let Script::HudMessage(h) = &mut self.s.scripts[sc as usize] {
            if !h.not_one_time {
                h.destroyed = true;
                self.s.invokes.retain(|i| i.script != sc);
            }
        }
    }

    /// InputManager.GetBindingString, "NO BINDING" when empty.
    fn binding_or_none(&self, action: &str) -> String {
        let s = self.input_actions.as_ref().map(|a| a.binding_string(action)).unwrap_or_default();
        if s.is_empty() {
            "NO BINDING".into()
        } else {
            s
        }
    }

    /// HudMessage.PlayMessage(hasToBeEnabled)
    pub(crate) fn hud_message_play(&mut self, sc: u32, has_to_be_enabled: bool) {
        let live = self.script_live(sc);
        let Script::HudMessage(h) = &mut self.s.scripts[sc as usize] else { return };
        if h.destroyed {
            return;
        }
        if h.deactivating {
            self.hud_message_done(sc);
            return;
        }
        if (h.activated && !h.not_one_time) || (has_to_be_enabled && !live) {
            return;
        }
        h.activated = true;
        let h = (**h).clone();
        if h.advanced {
            let inputs = h.actions.iter().map(|a| self.binding_or_none(a.as_deref().unwrap_or(""))).collect();
            self.send_hud_message2(&h.message, Some(inputs), 0.0, h.silent, true, false);
        } else if let Some(a) = &h.action {
            let b = self.binding_or_none(a);
            self.send_hud_message_full(&h.message, &b, &h.message2, 0.0, h.silent, true, false);
        } else {
            self.send_hud_message_full(&h.message, "", &h.message2, 0.0, h.silent, true, false);
        }
        if h.timed && h.not_one_time {
            self.cancel_invoke(sc, Act::HudMessageDone);
            self.invoke(sc, Act::HudMessageDone, h.timer);
        } else if h.timed {
            self.invoke(sc, Act::HudMessageDone, h.timer);
        } else if !h.deactivate_on_exit && !h.deactive_on_disable {
            self.invoke(sc, Act::HudMessageBegone, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::net_format;

    #[test]
    fn format() {
        let a = vec!["RMB".to_string()];
        assert_eq!(net_format("Hold <color=orange>{0}</color> to charge", &a).as_deref(), Some("Hold <color=orange>RMB</color> to charge"));
        assert_eq!(net_format("{{x}} {0}", &a).as_deref(), Some("{x} RMB"));
        assert_eq!(net_format("{1}", &a), None);
        assert_eq!(net_format("a } b", &a), None);
    }
}
