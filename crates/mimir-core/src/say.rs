//! What Mimir itself says in Telegram, Slack and Discord (pairing, approval cards, errors, the digest, the weekly
//! review), in the language picked in Settings, or the system's. Bots answer in the person's own language either way.
//!
//! Strings are looked up by their English: `tr("Wrong pairing code.")`, or `trf` with `{name}` placeholders. Anything
//! without a row stays English. Adding a language: one entry in `LANGS` and one table like `NL`.

use std::sync::atomic::{AtomicUsize, Ordering};

/// (code, its own name) per language, English first.
pub const LANGS: &[(&str, &str)] = &[("en", "English"), ("nl", "Nederlands")];

fn table(lang: usize) -> &'static [(&'static str, &'static str)] {
    match LANGS[lang].0 {
        "nl" => NL,
        _ => &[],
    }
}

// ponytail: one language per process (one Db per process); per-chat languages if someone asks
static LANG: AtomicUsize = AtomicUsize::new(0);

/// The setting `language` ("" = the system's), as a code from `LANGS`.
pub fn chosen(db: &crate::db::Db) -> String {
    let set = db.setting("language").ok().flatten().unwrap_or_default();
    if LANGS.iter().any(|(c, _)| *c == set) { set } else { system() }
}

/// Use the language in the settings from now on (at startup, and when it's changed).
pub fn load(db: &crate::db::Db) {
    let code = chosen(db);
    LANG.store(LANGS.iter().position(|(c, _)| *c == code).unwrap_or(0), Ordering::Relaxed);
}

/// The system's language if Mimir speaks it, else English.
pub fn system() -> String {
    // Tests check the English, whatever the machine they run on speaks.
    if cfg!(test) {
        return "en".into();
    }
    // A Mac app opened from the Finder has no LANG: ask macOS for the first preferred language.
    #[cfg(target_os = "macos")]
    let mac = std::process::Command::new("defaults").args(["read", "-g", "AppleLanguages"]).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    #[cfg(not(target_os = "macos"))]
    let mac: Option<String> = None;
    // ponytail: Windows follows LANG only (rarely set there); the setting covers it
    let env = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().filter_map(|v| std::env::var(v).ok()).find(|v| !v.is_empty() && v != "C" && v != "POSIX" && !v.starts_with("C."));
    let tag = mac.and_then(|m| m.split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_').find(|w| !w.is_empty()).map(str::to_string)).or(env).unwrap_or_default();
    let code = tag.get(..2).unwrap_or("").to_ascii_lowercase();
    if LANGS.iter().any(|(c, _)| *c == code) { code } else { "en".into() }
}

/// This English string in the current language.
pub fn tr(en: &'static str) -> &'static str {
    tr_in(LANG.load(Ordering::Relaxed), en)
}

fn tr_in(lang: usize, en: &'static str) -> &'static str {
    table(lang).iter().find(|(k, _)| *k == en).map_or(en, |(_, t)| t)
}

/// The same, with `{name}` placeholders filled in.
pub fn trf(en: &'static str, args: &[(&str, &str)]) -> String {
    fill(tr(en), args)
}

/// One pass, so a value holding braces (a memory's title, what was heard) is left alone.
fn fill(t: &str, args: &[(&str, &str)]) -> String {
    let mut out = String::new();
    let mut rest = t;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 1..];
        match tail.find('}').and_then(|j| args.iter().find(|(k, _)| *k == &tail[..j]).map(|(_, v)| (j, v))) {
            Some((j, v)) => {
                out.push_str(v);
                rest = &tail[j + 1..];
            }
            None => {
                out.push('{');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

static NL: &[(&str, &str)] = &[
    // Pairing.
    ("I can only be paired by a person writing as themselves (not anonymously or as a channel).", "Alleen iemand die als zichzelf schrijft kan me koppelen (niet anoniem of als kanaal)."),
    (
        "Paired ✅ as a guest: I keep your own memory and reminders here, and I can search the web for you. I can't see the email, calendar or meetings of the person who runs me. Commands: /memory, /routines, /new, /stop",
        "Gekoppeld ✅ als gast: ik bewaar hier je eigen geheugen en herinneringen, en ik kan voor je op het web zoeken. De e-mail, agenda en vergaderingen van degene die mij beheert zie ik niet. Commando's: /memory, /routines, /new, /stop",
    ),
    ("Paired ✅ Talk to me normally. Commands: /memory, /routines, /new, /stop, /undo, /why", "Gekoppeld ✅ Praat gewoon met me. Commando's: /memory, /routines, /new, /stop, /undo, /why"),
    ("Wrong pairing code. Too many tries: try again in an hour.", "Verkeerde koppelcode. Te vaak geprobeerd: probeer het over een uur opnieuw."),
    ("Wrong pairing code.", "Verkeerde koppelcode."),
    ("this bot", "deze bot"),
    (
        "Hi! I'm {name}, a private assistant. If you set me up, send pair followed by the code shown on my page in the Mimir app.",
        "Hoi! Ik ben {name}, een privé-assistent. Heb jij mij ingesteld? Stuur dan pair, gevolgd door de code op mijn pagina in de Mimir-app.",
    ),
    (
        "For your safety I now answer only the person who pairs me in a group. Whoever set me up: send pair followed by the code on my page in the Mimir app, here, once more.",
        "Voor je veiligheid antwoord ik in een groep alleen nog degene die mij koppelt. Wie mij heeft ingesteld: stuur hier nog één keer pair, gevolgd door de code op mijn pagina in de Mimir-app.",
    ),
    // Commands.
    ("Nothing to stop: I'm not writing anything right now.", "Niets te stoppen: ik ben nu niets aan het schrijven."),
    ("I've used today's share for you, sorry. I'm back tomorrow, or sooner if the person who runs me allows more today.", "Sorry, je deel voor vandaag is op. Morgen ben ik er weer, of eerder als degene die mij beheert vandaag meer toestaat."),
    (
        "Hi! Just talk to me. /memory shows what I remember, /routines lists schedules, /why shows what my last reply used, /new starts a fresh conversation, /branch (as a reply to a message) starts a new one from there, /stop cuts off a reply I'm writing, /undo N takes back something from my weekly list of what I learned.",
        "Hoi! Praat gewoon met me. /memory laat zien wat ik onthoud, /routines toont wat ik volgens schema doe, /why laat zien wat mijn laatste antwoord gebruikte, /new begint een nieuw gesprek, /branch (als antwoord op een bericht) begint er een nieuw gesprek vanaf, /stop breekt een antwoord af dat ik aan het schrijven ben, /undo N neemt iets terug uit mijn wekelijkse lijst van wat ik heb geleerd.",
    ),
    ("Not confirmed yet (saved after reading outside content; check them in the app):", "Nog niet bevestigd (opgeslagen na het lezen van inhoud van buitenaf; bekijk ze in de app):"),
    ("Nothing remembered yet.", "Nog niets onthouden."),
    ("My last reply didn't use any memories, skills or pages.", "Mijn laatste antwoord gebruikte geen herinneringen, skills of pagina's."),
    ("My last reply used:", "Mijn laatste antwoord gebruikte:"),
    (" (forgotten since)", " (inmiddels vergeten)"),
    (" (corrected since)", " (inmiddels aangepast)"),
    ("• skill: {name}", "• skill: {name}"),
    (
        "\nIf a memory is wrong, tell me (\"forget #{id}\", or what's true now), or fix it in the app under Memory.",
        "\nKlopt een herinnering niet? Zeg het me (\"vergeet #{id}\", of wat nu klopt), of pas het aan in de app onder Memory.",
    ),
    ("Fresh conversation started (memories kept).", "Nieuw gesprek begonnen (herinneringen blijven bewaard)."),
    ("Branch", "Aftakking"),
    ("Branched into a new topic; this one carries on as it was.", "Afgetakt naar een nieuw onderwerp; dit gesprek gaat gewoon door."),
    ("New conversation, picking up from there (memories kept). Where shall we take it?", "Nieuw gesprek, verder vanaf daar (herinneringen blijven bewaard). Welke kant gaan we op?"),
    ("Branch off a new conversation from here (reply to a message to branch from it)", "Begin hier een nieuw gesprek (antwoord op een bericht om vanaf dat bericht te beginnen)"),
    ("_Stopped before replying._", "_Gestopt voordat ik antwoordde._"),
    ("_Stopped._", "_Gestopt._"),
    ("Sorry for the slow reply: I was offline when you wrote {ago}.", "Sorry voor het late antwoord: ik was offline toen je {ago} schreef."),
    ("{n} minutes ago", "{n} minuten geleden"),
    ("an hour ago", "een uur geleden"),
    ("{n} hours ago", "{n} uur geleden"),
    ("yesterday", "gisteren"),
    ("{n} days ago", "{n} dagen geleden"),
    // Files and voice notes.
    (
        "I can read text, photos, voice notes, PDFs, Word and Excel files and text files, but not this kind of file yet. Could you describe it or send a photo?",
        "Ik kan tekst, foto's, spraakberichten, pdf's, Word- en Excel-bestanden en tekstbestanden lezen, maar dit soort bestand nog niet. Kun je het beschrijven of er een foto van sturen?",
    ),
    ("That file has no text I can read (a scanned PDF?). Send photos of the pages instead.", "In dat bestand staat geen tekst die ik kan lezen (een gescande pdf?). Stuur liever foto's van de pagina's."),
    ("That file is over 20 MB, more than I take. Could you send a smaller one?", "Dat bestand is groter dan 20 MB, meer dan ik aanneem. Kun je een kleiner bestand sturen?"),
    ("That file is over 20 MB, more than Telegram lets me download. Could you send a smaller one?", "Dat bestand is groter dan 20 MB, meer dan Telegram mij laat downloaden. Kun je een kleiner bestand sturen?"),
    ("I couldn't fetch {name} ({error}).", "Ik kon {name} niet ophalen ({error})."),
    ("🎙️ I heard: “{text}”", "🎙️ Ik hoorde: “{text}”"),
    ("I couldn't make out any words in that voice note.", "Ik kon geen woorden verstaan in dat spraakbericht."),
    ("I couldn't transcribe that voice note ({error}). Could you type it?", "Ik kon dat spraakbericht niet uitschrijven ({error}). Kun je het typen?"),
    (
        "To understand voice notes I need whisper.cpp where Mimir runs, or an OpenAI API key (Settings → AI keys in Mimir). Until then, could you type it?",
        "Om spraakberichten te verstaan heb ik whisper.cpp nodig waar Mimir draait, of een OpenAI-API-sleutel (Settings → AI keys in Mimir). Kun je het tot die tijd typen?",
    ),
    // Approval cards.
    ("Approve", "Goedkeuren"),
    ("Deny", "Weigeren"),
    ("Approve all like this", "Alles zoals dit goedkeuren"),
    ("Done", "Klaar"),
    ("Can't now", "Lukt nu niet"),
    ("Always on {site}", "Altijd op {site}"),
    ("Always: {tool}", "Altijd: {tool}"),
    ("Always approve plans", "Plannen altijd goedkeuren"),
    ("{n} h", "{n} uur"),
    ("{n} min", "{n} min"),
    ("It runs when you approve.", "Het gebeurt zodra je het goedkeurt."),
    ("No answer means deny.", "Geen antwoord betekent nee."),
    ("You've allowed this {n} times. Press Always to stop being asked.", "Je hebt dit al {n} keer toegestaan. Druk op Altijd om het niet meer gevraagd te krijgen."),
    ("🖐 I need you on the computer\n\n{summary}\n\nWaits {wait}.", "🖐 Ik heb je nodig aan de computer\n\n{summary}\n\nWacht {wait}."),
    ("🔐 Approve this action?\n\n{summary}\n\nWaits {wait}. {then}", "🔐 Deze actie goedkeuren?\n\n{summary}\n\nWacht {wait}. {then}"),
    ("✅ Done, carrying on", "✅ Klaar, ik ga verder"),
    ("⌛ Nobody took over in time", "⌛ Niemand nam het op tijd over"),
    ("Not now", "Nu niet"),
    ("🔑 I need you to sign in\n\n{summary}\n\nWaits {wait}.", "🔑 Je moet even inloggen\n\n{summary}\n\nWacht {wait}."),
    ("✅ Signed in", "✅ Ingelogd"),
    ("⌛ Nobody signed in in time", "⌛ Niemand logde op tijd in"),
    ("❌ Not signed in", "❌ Niet ingelogd"),
    ("✅ Approved", "✅ Goedgekeurd"),
    ("⌛ No answer in time, so not done", "⌛ Geen antwoord op tijd, dus niet gedaan"),
    ("❌ Denied", "❌ Geweigerd"),
    ("Why: {why}", "Waarom: {why}"),
    ("Reply to this message to say no and why.", "Antwoord op dit bericht om te weigeren en te zeggen waarom."),
    ("Edit in Mimir", "Bewerken in Mimir"),
    ("From now on this runs on {site} without asking (Settings → Permissions to change it).", "Voortaan gebeurt dit op {site} zonder te vragen (te wijzigen in Settings → Permissions)."),
    ("From now on this runs without asking (Settings → Permissions to change it).", "Voortaan gebeurt dit zonder te vragen (te wijzigen in Settings → Permissions)."),
    ("Only the person who paired me here can answer this.", "Alleen degene die mij hier heeft gekoppeld kan dit beantwoorden."),
    ("This request is no longer pending.", "Dit verzoek staat niet meer open."),
    ("Unknown button.", "Onbekende knop."),
    // Telegram's "/" menu.
    ("What I remember about you", "Wat ik over je onthoud"),
    ("What I do on a schedule", "Wat ik volgens schema doe"),
    ("What my last reply used", "Wat mijn laatste antwoord gebruikte"),
    ("Start a fresh conversation (memories kept)", "Begin een nieuw gesprek (herinneringen blijven bewaard)"),
    ("Stop the reply I'm writing", "Stop het antwoord dat ik schrijf"),
    // Slack's status line while a reply is written ("<bot> is thinking…").
    ("is thinking…", "denkt na…"),
    // Errors.
    (
        "My Claude plan won't cover this: Anthropic is counting it as paid extra usage, and there's none left.",
        "Mijn Claude-abonnement dekt dit niet: Anthropic rekent het als betaald extra gebruik, en dat is op.",
    ),
    (
        "Turn on or top up extra usage in your Claude account, or give me another AI account in Mimir (ChatGPT, or an API key).",
        "Zet extra gebruik aan of vul het aan in je Claude-account, of geef me een ander AI-account in Mimir (ChatGPT, of een API-sleutel).",
    ),
    ("I've hit the AI plan's usage limit.", "Ik zit aan de gebruikslimiet van het AI-abonnement."),
    ("Try again later, or give me another AI account or model in Mimir.", "Probeer het later nog eens, of geef me een ander AI-account of model in Mimir."),
    ("My AI account isn't signed in, or its key was rejected.", "Mijn AI-account is niet ingelogd, of de sleutel werd geweigerd."),
    ("Open Mimir and check my AI account (Settings, or my page).", "Open Mimir en controleer mijn AI-account (Settings, of mijn pagina)."),
    ("I couldn't start my AI.", "Ik kon mijn AI niet starten."),
    ("Open Mimir and check that my AI account is installed and signed in.", "Open Mimir en controleer of mijn AI-account geïnstalleerd en ingelogd is."),
    ("I couldn't reach my AI.", "Ik kon mijn AI niet bereiken."),
    ("If it's a local model, check it's running; otherwise try again in a minute.", "Is het een lokaal model, kijk dan of het draait; probeer het anders over een minuut opnieuw."),
    ("I don't have a model chosen.", "Er is geen model voor mij gekozen."),
    ("(The model {model} isn't offered any more, so I used the default. Pick another on my page in Mimir.)", "(Het model {model} wordt niet meer aangeboden, dus ik heb het standaardmodel gebruikt. Kies een ander op mijn pagina in Mimir.)"),
    ("Open my page in Mimir and pick one.", "Open mijn pagina in Mimir en kies er een."),
    ("That took too long, so I stopped.", "Dat duurde te lang, dus ik ben gestopt."),
    ("Try again, maybe with a smaller ask.", "Probeer het opnieuw, misschien met een kleinere vraag."),
    ("Something went wrong with that reply.", "Er ging iets mis met dat antwoord."),
    ("Try again; if it keeps happening, the details are in Mimir's Needs you list.", "Probeer het opnieuw; blijft het gebeuren, dan staan de details in Mimirs lijst Needs you."),
    ("Routines on this AI account wait", "Routines op dit AI-account wachten"),
    ("This bot's routines wait", "De routines van deze bot wachten"),
    ("A task I handed to another bot is done: its answer is in the run log.", "Een taak die ik aan een andere bot gaf is klaar: het antwoord staat in het runlogboek."),
    ("⏸ I've hit my AI plan's usage limit. {who} until {at}, when the limit resets; this one runs again then.", "⏸ Ik zit aan de gebruikslimiet van mijn AI-abonnement. {who} tot {at}, als de limiet weer vrij is; deze draait dan opnieuw."),
    // Buttons under a question, a reminder or a routine's result, and what a tap says.
    ("Something else", "Iets anders"),
    ("Snooze 1h", "Over 1 uur"),
    ("Tomorrow", "Morgen"),
    ("Less like this", "Minder zoals dit"),
    ("Pause", "Pauzeren"),
    ("For tomorrow…", "Voor morgen…"),
    ("This chat isn't paired any more.", "Deze chat is niet meer gekoppeld."),
    ("Already done.", "Al gedaan."),
    ("(Ask me in our private chat for the rest.)", "(Vraag me in onze privéchat om de rest.)"),
    ("These buttons have expired. Ask me again if you still need it.", "Deze knoppen zijn verlopen. Vraag het me opnieuw als je het nog nodig hebt."),
    ("You chose: {choice}", "Je koos: {choice}"),
    ("✍️ Type your answer: {question}", "✍️ Typ je antwoord: {question}"),
    ("Type your answer.", "Typ je antwoord."),
    ("(Reply with a number.)", "(Antwoord met een nummer.)"),
    ("(Reply with a number. If I've written since, reply to this message with it.)", "(Antwoord met een nummer. Heb ik intussen iets anders gestuurd, beantwoord dan dit bericht met het nummer.)"),
    ("✅ Done", "✅ Klaar"),
    ("⏰ I'll remind you again today at {at}.", "⏰ Ik herinner je er vandaag om {at} weer aan."),
    ("⏰ I'll remind you again tomorrow at {at}.", "⏰ Ik herinner je er morgen om {at} weer aan."),
    ("The plan resets at {at}.", "De limiet is om {at} weer vrij."),
    ("Answer at {at}", "Antwoord om {at}"),
    ("Answer now with {account}", "Antwoord nu met {account}"),
    ("⏰ I'll answer at {at}.", "⏰ Ik antwoord om {at}."),
    ("Answering with {account}…", "Ik antwoord met {account}…"),
    ("This bot has no fallback account any more.", "Deze bot heeft geen reserve-account meer."),
    ("👍 Noted.", "👍 Genoteerd."),
    ("👎 Noted: I'll learn from it.", "👎 Genoteerd: ik leer ervan."),
    ("Noted: less like this.", "Genoteerd: minder zoals dit."),
    ("⏸ Paused routine #{id}. Ask me, or use Mimir, to start it again.", "⏸ Routine #{id} gepauzeerd. Vraag het mij, of gebruik Mimir, om hem weer te starten."),
    ("That routine is gone.", "Die routine bestaat niet meer."),
    ("What should be different next time? Reply here and I'll change routine #{id}.", "Wat moet er volgende keer anders? Antwoord hier en ik pas routine #{id} aan."),
    ("Tell me what to change.", "Vertel me wat ik moet veranderen."),
    // The decisions chat.
    ("From: {chat}", "Uit: {chat}"),
    ("From {chat}: {text}", "Uit {chat}: {text}"),
    ("🧠 1 memory suggestion for {chat}: accept or dismiss it in Mimir's Needs you.", "🧠 1 voorstel voor het geheugen van {chat}: neem het aan of wijs het af in Mimirs Needs you."),
    ("🧠 {n} memory suggestions for {chat}: accept or dismiss them in Mimir's Needs you.", "🧠 {n} voorstellen voor het geheugen van {chat}: neem ze aan of wijs ze af in Mimirs Needs you."),
    // The digest.
    ("☀️ **All quiet**", "☀️ **Alles rustig**"),
    ("☀️ **Your digest**", "☀️ **Je overzicht**"),
    ("A routine", "Een routine"),
    ("1 quiet check ran; nothing needed you.", "1 stille controle gedaan; niets had je nodig."),
    ("{n} quiet checks ran; nothing needed you.", "{n} stille controles gedaan; niets had je nodig."),
    // The weekly review.
    ("🧠 **What I learned this week**", "🧠 **Wat ik deze week heb geleerd**"),
    ("You said: “{said}”", "Jij zei: “{said}”"),
    (
        "Wrong? Reply /undo {n} to take it back (or /undo with another number). It's all in Memory in the app too.",
        "Klopt iets niet? Antwoord /undo {n} om het terug te nemen (of /undo met een ander nummer). Alles staat ook onder Memory in de app.",
    ),
    ("This week: {trends}", "Deze week: {trends}"),
    ("{n} corrections per 10 messages", "{n} correcties per 10 berichten"),
    ("{n} corrections per 10 messages (was {was})", "{n} correcties per 10 berichten (was {was})"),
    ("{n} repeated an earlier correction", "{n} keer een eerdere correctie herhaald"),
    ("{n} \"I told you\"", "{n} keer \"dat heb ik je al gezegd\""),
    ("{denied} denied, {stopped} stopped", "{denied} geweigerd, {stopped} gestopt"),
    ("{n}% of actions failed", "{n}% van de acties mislukt"),
    ("{n} of {all} suggestions accepted", "{n} van {all} voorstellen aangenomen"),
    ("1 learned memory taken back", "1 geleerde herinnering teruggenomen"),
    ("{n} learned memories taken back", "{n} geleerde herinneringen teruggenomen"),
    ("There's nothing to undo yet: I list what I learned once a week.", "Er is nog niets terug te nemen: ik zet één keer per week op een rij wat ik heb geleerd."),
    ("Pick a number from 1 to {n}.", "Kies een nummer van 1 tot en met {n}."),
    ("Took back “{title}”.", "“{title}” teruggenomen."),
    ("“{title}” was already changed or forgotten.", "“{title}” was al aangepast of vergeten."),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The English strings handed to `tr`/`trf` in the source, as written there.
    fn used() -> Vec<String> {
        // Every .rs file under src/, subfolders (db/) included.
        let (mut dirs, mut files) = (vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")], vec![]);
        while let Some(dir) = dirs.pop() {
            for p in std::fs::read_dir(dir).unwrap().flatten().map(|f| f.path()) {
                if p.is_dir() { dirs.push(p) } else { files.push(p) }
            }
        }
        let mut out = vec![];
        for f in files.iter().filter(|p| !p.ends_with("say.rs")) {
            let src = std::fs::read_to_string(f).unwrap();
            let calls = src.match_indices("tr(\"").chain(src.match_indices("trf(\"")).filter(|(i, _)| !src[..*i].ends_with(|c: char| c.is_alphanumeric() || c == '_'));
            for (i, m) in calls {
                let part = &src[i + m.len()..];
                // Up to the closing quote (an escaped \" doesn't end it).
                let end = part.char_indices().find(|&(i, c)| c == '"' && !part[..i].ends_with('\\')).map_or(part.len(), |(i, _)| i);
                out.push(part[..end].replace("\\n", "\n").replace("\\\"", "\""));
            }
        }
        out
    }

    #[test]
    fn every_string_has_a_translation_with_the_same_placeholders() {
        let holes = |s: &str| {
            let mut h: Vec<String> = s.split('{').skip(1).filter_map(|p| p.split_once('}').map(|(k, _)| k.to_string())).collect();
            h.sort();
            h
        };
        let used = used();
        assert!(used.len() > 50, "found only {} tr calls", used.len());
        for en in &used {
            for (lang, (code, _)) in LANGS.iter().enumerate().skip(1) {
                let row = table(lang).iter().find(|(k, _)| k == en);
                assert!(row.is_some(), "no {code} for {en:?}");
                assert_eq!(holes(en), holes(row.unwrap().1), "{en:?}");
            }
        }
        for (en, _) in NL {
            assert!(used.iter().any(|u| u == en), "unused row {en:?}");
        }
    }

    #[test]
    fn fills_placeholders_once() {
        assert_eq!(trf("Took back “{title}”.", &[("title", "{n} x")]), "Took back “{n} x”.");
        assert_eq!(trf("{n} min", &[]), "{n} min");
        assert_eq!(fill(tr_in(1, "Pick a number from 1 to {n}."), &[("n", "3")]), "Kies een nummer van 1 tot en met 3.");
    }
}
