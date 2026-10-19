//! Original starter presentations for File › New.
//!
//! Each deck is a few slides of our own text on one of our themes. Nothing here is copied from
//! another product's templates.

use std::sync::Arc;

use deckcraft_geom::Size;
use deckcraft_model::text::TextBody;
use deckcraft_model::{Presentation, defaults};
use serde_json::Value;

use crate::Result;
use crate::cmd::{bad, slide::find_layout};

/// One slide of a starter: a layout, a title, text for each non-title placeholder in order, and
/// optional speaker notes.
pub struct SlideSeed {
    pub layout: &'static str,
    pub title: &'static str,
    pub bodies: &'static [&'static str],
    pub notes: &'static str,
}

/// A starter deck shown in the template gallery.
pub struct Starter {
    pub name: &'static str,
    pub summary: &'static str,
    pub theme: &'static str,
    /// The gallery category (one of [`CATEGORIES`]).
    pub category: &'static str,
    /// Extra search words: audiences, occasions and styles this deck suits.
    pub tags: &'static [&'static str],
    pub slides: &'static [SlideSeed],
}

/// The template gallery's categories, in the order File › New shows them.
pub const CATEGORIES: [&str; 10] = [
    "Business",
    "Education",
    "Sales & Marketing",
    "Reports & Research",
    "Portfolio",
    "Events & Talks",
    "Travel & Outdoors",
    "Community & Nonprofit",
    "Technology",
    "Creative",
];

const fn seed(layout: &'static str, title: &'static str, bodies: &'static [&'static str], notes: &'static str) -> SlideSeed {
    SlideSeed { layout, title, bodies, notes }
}

const STARTERS: &[Starter] = &[
    Starter {
        name: "Briefing",
        summary: "A weekly check-in: what changed, what's stuck, and who does the next thing",
        theme: "Harbor",
        category: "Business",
        tags: &["weekly", "team", "status", "meeting", "update"],
        slides: &[
            seed("title", "Weekly briefing", &["What changed, what's stuck, and what we do next"], "Open with what changed since last week."),
            seed(
                "titleAndContent",
                "This week's agenda",
                &["Shipped since last time\n\tOne concrete change\nStill open\n\tThe decision we need\nNext\n\tWho does it, and by when"],
                "",
            ),
            seed("sectionHeader", "Where we are", &["A short picture before the details"], ""),
            seed(
                "twoContent",
                "Done and next",
                &[
                    "Done\n\tThe change is in people's hands\n\tWe wrote down what we learned",
                    "Next\n\tPick the one risk to retire\n\tSay who owns it",
                ],
                "",
            ),
            seed("title", "See you next week", &["Bring one number and one question"], ""),
        ],
    },
    Starter {
        name: "Lesson",
        summary: "A short class that ends with the learner explaining the idea",
        theme: "Meadow",
        category: "Education",
        tags: &["teaching", "class", "workshop", "explainer", "students"],
        slides: &[
            seed("title", "How a lock works", &["A short lesson for people new to the idea"], ""),
            seed(
                "titleAndContent",
                "By the end",
                &["You can say what a lock protects\n\tAnd what it does not\nYou can sketch one in three steps"],
                "",
            ),
            seed("sectionHeader", "The idea", &["One metaphor, then the real thing"], ""),
            seed(
                "titleAndContent",
                "Three steps",
                &["Name the thing being shared\nDecide who may change it\nLeave it so the next person can tell"],
                "",
            ),
            seed("title", "Try it", &["Explain a lock to someone who has never seen one"], ""),
        ],
    },
    Starter {
        name: "Pitch",
        summary: "A ten-minute conversation about one promise, not a tour of features",
        theme: "Ember",
        category: "Sales & Marketing",
        tags: &["startup", "investor", "sales", "product", "story"],
        slides: &[
            seed("title", "A quieter inbox", &["Ten minutes. One promise."], ""),
            seed(
                "titleAndContent",
                "The mess",
                &[
                    "People miss the one message that mattered\n\tIt arrived between newsletters\nThey spend the morning sorting\n\tAnd still feel behind",
                ],
                "",
            ),
            seed("sectionHeader", "A smaller promise", &["Show the one message. Hide the rest until asked."], ""),
            seed(
                "comparison",
                "Today and after",
                &[
                    "Today",
                    "The important note is buried\nFinding it takes a search",
                    "After",
                    "It sits at the top until you deal with it\nEverything else waits",
                ],
                "",
            ),
            seed("title", "The ask", &["Twenty people, two weeks, one question: did the right note surface?"], ""),
        ],
    },
    Starter {
        name: "Field Notes",
        summary: "What an outing set out to see, and what it actually found",
        theme: "Coral Reef",
        category: "Travel & Outdoors",
        tags: &["travel", "hiking", "journal", "nature", "fieldwork"],
        slides: &[
            seed("title", "Saturday on the ridge", &["What we went to see, and what we found"], ""),
            seed("titleAndContent", "Plan", &["Leave at dawn\n\tCold, clear, little wind\nWalk the west path\nBe back before dusk"], ""),
            seed("sectionHeader", "What we saw", &["Notes taken on the spot"], ""),
            seed(
                "twoContent",
                "Expected and found",
                &[
                    "Expected\n\tA clear view from the ridge\n\tThe creek on the map",
                    "Found\n\tLow cloud on the ridge\n\tThe creek, and a better pencil",
                ],
                "",
            ),
            seed("title", "Pack list for next time", &["Warmer gloves. A second pencil. The map that shows the creek."], ""),
        ],
    },
    Starter {
        name: "Quarterly",
        summary: "A report meant to be read aloud in half an hour",
        theme: "Paper",
        category: "Reports & Research",
        tags: &["report", "quarterly", "business", "numbers", "review"],
        slides: &[
            seed("title", "Spring report", &["Read aloud in thirty minutes"], ""),
            seed(
                "titleAndContent",
                "Headlines",
                &["The waiting list got shorter\n\tBy the amount we promised\nTwo things slipped\n\tBoth have a new date"],
                "",
            ),
            seed("sectionHeader", "The numbers", &["Only the ones that change a decision"], ""),
            seed("titleAndContent", "What we will stop", &["The report nobody reads\nThe meeting that repeats the report"], ""),
            seed("title", "Questions we want", &["Where should the next quarter refuse work?"], ""),
        ],
    },
    Starter {
        name: "Evening Talk",
        summary: "A public talk the audience can check on the way home",
        theme: "Nocturne",
        category: "Events & Talks",
        tags: &["talk", "public", "evening", "conference", "lecture"],
        slides: &[
            seed("title", "Lights out", &["A talk about how cities sleep"], ""),
            seed("titleAndContent", "Road map", &["Why the sky looks orange\nWhat a dark street is for\nOne change a block can make"], ""),
            seed("sectionHeader", "Look up", &["The part you can check on the way home"], ""),
            seed(
                "titleAndContent",
                "One change",
                &["Shield the lamp so light goes down\n\tNot into bedroom windows\nLeave a path you can still walk"],
                "",
            ),
            seed("title", "Walk home", &["Count the lamps you can see from your pillow"], ""),
        ],
    },
    Starter {
        name: "Project Kickoff",
        summary: "The first meeting of a project: the promise, the first fortnight, and who decides",
        theme: "Harbor",
        category: "Business",
        tags: &["project", "kickoff", "planning", "team", "scope"],
        slides: &[
            seed("title", "Project Northstar", &["What we are making, and what we are not"], "Name the one outcome the project exists for."),
            seed(
                "titleAndContent",
                "The promise",
                &["A person can do the thing in one sitting\n\tWithout reading a manual\nWe know it works when they come back"],
                "",
            ),
            seed("sectionHeader", "The first fortnight", &["Small enough to finish, real enough to learn from"], ""),
            seed(
                "twoContent",
                "Decisions and open questions",
                &[
                    "Decided\n\tThe scope is the one journey\n\tWe ship behind a flag",
                    "Open\n\tWho owns the support queue?\n\tWhat would make us stop?",
                ],
                "Leave with an owner for every open question.",
            ),
            seed("title", "Next meeting", &["Bring a demo, not a status report"], ""),
        ],
    },
    Starter {
        name: "Study Guide",
        summary: "A revision deck that turns a chapter into questions, examples and a teach-back",
        theme: "Meadow",
        category: "Education",
        tags: &["study", "revision", "exam", "school", "learning"],
        slides: &[
            seed("title", "Photosynthesis, step by step", &["A revision guide you can teach from"], ""),
            seed(
                "titleAndContent",
                "What to know",
                &["Where the energy comes from\n\tLight, not soil\nWhat goes in and what comes out\nWhy the leaf is shaped the way it is"],
                "Ask the class to say the inputs before showing them.",
            ),
            seed("sectionHeader", "Worked example", &["Trace one carbon atom from air to sugar"], ""),
            seed(
                "twoContent",
                "Practice and check",
                &[
                    "Try\n\tSketch the two stages\n\tLabel where each one happens",
                    "Check\n\tDid you place the light reactions first?\n\tDid you account for the oxygen?",
                ],
                "",
            ),
            seed("title", "Teach it back", &["Explain it to someone who missed the lesson, in two minutes"], ""),
        ],
    },
    Starter {
        name: "Product Launch",
        summary: "A launch brief for the team: who it is for, the one change, and the first week",
        theme: "Ember",
        category: "Sales & Marketing",
        tags: &["launch", "product", "marketing", "release", "campaign"],
        slides: &[
            seed("title", "A calmer morning", &["Launch brief for the team"], ""),
            seed(
                "titleAndContent",
                "Who it is for",
                &[
                    "People who open the same three tools before coffee\n\tAnd still miss the one update that mattered\nNot everyone: we are not building for everyone",
                ],
                "",
            ),
            seed("sectionHeader", "The one change", &["The update finds them, not the other way round"], ""),
            seed(
                "comparison",
                "Before and after",
                &[
                    "Before",
                    "Three tabs, two searches, one missed message\nThe morning starts behind",
                    "After",
                    "One place that shows what changed\nEverything else waits its turn",
                ],
                "",
            ),
            seed("title", "Launch day", &["One demo, one email, one question: did the right update surface?"], ""),
        ],
    },
    Starter {
        name: "Research Findings",
        summary: "What a study asked, what the evidence supports, and where it stops",
        theme: "Slate",
        category: "Reports & Research",
        tags: &["research", "study", "findings", "evidence", "analysis"],
        slides: &[
            seed("title", "How people find the right tab", &["Findings from twelve sessions"], ""),
            seed(
                "titleAndContent",
                "What we asked",
                &[
                    "Can people return to a page they used yesterday?\n\tWithout searching for it\nWhat do they remember: the name, the place, or the colour?",
                ],
                "",
            ),
            seed("sectionHeader", "What we found", &["Three patterns, one surprise"], ""),
            seed(
                "twoContent",
                "Evidence and limits",
                &[
                    "Evidence\n\tTen of twelve used place before name\n\tColour helped only after a week",
                    "Limits\n\tTwelve people, one task\n\tWe did not test a second language",
                ],
                "Say what would change our mind.",
            ),
            seed("title", "What to do next", &["Test the same task with place-based labels"], ""),
        ],
    },
    Starter {
        name: "Case Study",
        summary: "A portfolio piece: the brief, the choices, and what the work changed",
        theme: "Terracotta",
        category: "Portfolio",
        tags: &["portfolio", "case study", "design", "client", "project"],
        slides: &[
            seed("title", "A library that answers back", &["Case study: six weeks, one service desk"], ""),
            seed(
                "titleAndContent",
                "The brief",
                &["People could find the building but not the help\n\tThe desk was behind a pillar\nThe staff answered the same five questions"],
                "",
            ),
            seed("sectionHeader", "What we made", &["A sign, a script, and a shorter queue"], ""),
            seed(
                "twoContent",
                "Choices and results",
                &[
                    "Choices\n\tPut the questions on the wall\n\tLet people join the queue from the door",
                    "Results\n\tThe five questions became a poster\n\tThe queue moved out of the rain",
                ],
                "",
            ),
            seed("title", "What we would keep", &["Ask the desk what it repeats before designing anything"], ""),
        ],
    },
    Starter {
        name: "Conference Session",
        summary: "A forty-minute session with one idea, one demo and three things to try",
        theme: "Nocturne",
        category: "Events & Talks",
        tags: &["conference", "talk", "session", "speaker", "workshop"],
        slides: &[
            seed("title", "Small tools, long tables", &["A session about software that gets out of the way"], ""),
            seed(
                "titleAndContent",
                "What you will leave with",
                &["One way to spot a tool that fits\nOne demo you can copy\nThree things to try on Monday"],
                "",
            ),
            seed("sectionHeader", "The middle", &["Show the work, not the slideware"], ""),
            seed(
                "titleAndContent",
                "Three things to try",
                &["Measure the wait, not the click\n\tThe queue is the product\nRemove one step from the first run\nSay no to the second dashboard"],
                "",
            ),
            seed("title", "Thank you", &["Questions, or come and show me your smallest tool"], ""),
        ],
    },
    Starter {
        name: "Community Update",
        summary: "A short update for neighbours: what changed, who did it, and how to join",
        theme: "Orchard",
        category: "Community & Nonprofit",
        tags: &["community", "nonprofit", "volunteers", "neighbours", "update"],
        slides: &[
            seed("title", "The garden gate is open", &["A spring update from the community plot"], ""),
            seed(
                "titleAndContent",
                "What changed",
                &[
                    "The gate no longer sticks\n\tA neighbour fixed the hinge\nThere are raised beds for anyone who wants one\nTools live in the green box",
                ],
                "",
            ),
            seed("sectionHeader", "Who made it happen", &["Six people, four Saturdays, one borrowed drill"], ""),
            seed(
                "twoContent",
                "What we need and what we have",
                &[
                    "We have\n\tCompost, seeds and a water butt\n\tA key for every household",
                    "We need\n\tSomeone to open on Sundays\n\tA second wheelbarrow",
                ],
                "",
            ),
            seed("title", "Join us", &["Come on Saturday at ten; bring gloves if you have them"], ""),
        ],
    },
    Starter {
        name: "Architecture Review",
        summary: "A technical review: the system's shape, where it hurts, and the trade-offs",
        theme: "Blueprint",
        category: "Technology",
        tags: &["architecture", "engineering", "systems", "review", "technical"],
        slides: &[
            seed("title", "The shape of the sync service", &["Architecture review: one queue, two stores"], ""),
            seed(
                "titleAndContent",
                "How it fits together",
                &[
                    "The editor writes a command journal\n\tCommands are the only mutation\nOne worker applies them to the document store\nReaders see a revision, never a half edit",
                ],
                "",
            ),
            seed("sectionHeader", "Where it hurts", &["The queue is the meeting point and the bottleneck"], ""),
            seed(
                "twoContent",
                "Options and trade-offs",
                &[
                    "Option A: one queue per document\n\tSimple ordering\n\tMore workers to run",
                    "Option B: one shared queue\n\tFewer moving parts\n\tA noisy document delays the rest",
                ],
                "Name the failure you are willing to operate.",
            ),
            seed("title", "The decision", &["One queue per document, with a shared dead-letter path"], ""),
        ],
    },
    Starter {
        name: "Incident Review",
        summary: "A blameless incident review: the timeline, what helped, and the one change",
        theme: "Monochrome",
        category: "Technology",
        tags: &["incident", "postmortem", "operations", "reliability", "engineering"],
        slides: &[
            seed("title", "The morning the exports stopped", &["Incident review, 14 March"], ""),
            seed(
                "titleAndContent",
                "What happened",
                &["Exports queued but never started\n\tA worker restart loop\nPeople saw a spinner with no end\n\tNo error reached the editor"],
                "State the user impact before the cause.",
            ),
            seed("sectionHeader", "The timeline", &["Fourteen minutes from first report to fix"], ""),
            seed(
                "twoContent",
                "What helped and what didn't",
                &[
                    "Helped\n\tThe queue depth graph\n\tOne engineer had run this before",
                    "Didn't\n\tThe retry alarm was too quiet\n\tThe runbook named the old dashboard",
                ],
                "",
            ),
            seed("title", "What changes", &["One alarm we can hear, one runbook we have walked through"], ""),
        ],
    },
    Starter {
        name: "Storyboard",
        summary: "A visual sequence: the idea in one line, beat by beat, look and sound",
        theme: "Coral Reef",
        category: "Creative",
        tags: &["storyboard", "film", "animation", "creative", "sequence"],
        slides: &[
            seed("title", "The last train home", &["A thirty-second storyboard"], ""),
            seed(
                "titleAndContent",
                "The idea in one line",
                &["A commuter misses the last train and finds the city waiting\n\tNot a disaster: an invitation"],
                "",
            ),
            seed("sectionHeader", "Beat by beat", &["Platform, clock, turn, street, window"], ""),
            seed(
                "twoContent",
                "Look and sound",
                &[
                    "Look\n\tWarm platform lights against blue night\n\tHandheld at the turn, still at the window",
                    "Sound\n\tThe announcement, then only footsteps\n\tA distant piano enters at the street",
                ],
                "",
            ),
            seed("title", "Next frame", &["Draw the turn: the moment the commuter stops running"], ""),
        ],
    },
    Starter {
        name: "Travel Journal",
        summary: "A trip told as a route: where, when, what we carried and what we would change",
        theme: "Paper",
        category: "Travel & Outdoors",
        tags: &["travel", "journal", "trip", "route", "holiday"],
        slides: &[
            seed("title", "Three days on the coast path", &["A short journal from the southwest"], ""),
            seed(
                "titleAndContent",
                "Where and when",
                &["Two nights in the harbour town\n\tArrive before the tide\nOne long walk east\n\tBack before the light goes"],
                "",
            ),
            seed("sectionHeader", "The walk", &["What the map left out"], ""),
            seed(
                "twoContent",
                "What we packed and what we wished for",
                &["Packed\n\tA flask, two maps, a repair kit\n\tThe small camera", "Wished for\n\tA second pair of socks\n\tA lighter rain jacket"],
                "",
            ),
            seed("title", "Go back", &["Next time: the west path in autumn, with the same flask"], ""),
        ],
    },
];

/// The starter gallery, in the order File › New shows it.
pub fn starters() -> &'static [Starter] {
    STARTERS
}

/// Starters whose name, summary, category, tags, theme or a slide title contains `query` (empty
/// matches all).
pub fn matching(query: &str) -> Vec<&'static Starter> {
    let q = query.trim().to_lowercase();
    let hit = |text: &str| text.to_lowercase().contains(&q);
    starters()
        .iter()
        .filter(|t| {
            q.is_empty()
                || hit(t.name)
                || hit(t.summary)
                || hit(t.category)
                || hit(t.theme)
                || t.tags.iter().any(|tag| hit(tag))
                || t.slides.iter().any(|slide| hit(slide.title))
        })
        .collect()
}

/// Starters in `category` (case-insensitive; `""` or `"all"` for every one).
pub fn by_category(category: &str) -> Vec<&'static Starter> {
    let c = category.trim();
    if c.is_empty() || c.eq_ignore_ascii_case("all") {
        return starters().iter().collect();
    }
    starters().iter().filter(|t| t.category.eq_ignore_ascii_case(c)).collect()
}

/// The gallery's categories, each with how many starters it holds, in gallery order. A category
/// with no starter is left out so the UI never shows an empty tab.
pub fn categories() -> Vec<(&'static str, usize)> {
    CATEGORIES.iter().map(|c| (*c, starters().iter().filter(|t| t.category == *c).count())).filter(|(_, n)| *n > 0).collect()
}

pub fn find(name: &str) -> Option<&'static Starter> {
    let name = name.trim();
    starters().iter().find(|t| t.name.eq_ignore_ascii_case(name))
}

/// A template the user saved: a `.deckcraft` file in the app's template folder.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
pub struct UserTemplate {
    pub name: String,
    pub path: std::path::PathBuf,
    pub theme: String,
    pub slides: usize,
}

/// The user templates in `dir`, by file name. A damaged file is listed with its name and no
/// theme/slides rather than hidden, so the user can see and remove it.
#[cfg(not(target_arch = "wasm32"))]
pub fn user_templates(dir: &std::path::Path) -> Vec<UserTemplate> {
    crate::io::app_files(dir)
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("deckcraft")))
        .filter_map(|path| {
            let name = path.file_stem()?.to_string_lossy().to_string();
            match crate::io::read_app_file(&path).ok().and_then(|b| deckcraft_format::load(&b).ok()) {
                Some(doc) => Some(UserTemplate {
                    name,
                    path,
                    theme: doc.masters.first().map(|m| m.theme.name.clone()).unwrap_or_default(),
                    slides: doc.slides.len(),
                }),
                None => Some(UserTemplate { name, path, theme: String::new(), slides: 0 }),
            }
        })
        .collect()
}

/// Load one user template file (the path listed by [`user_templates`]).
#[cfg(not(target_arch = "wasm32"))]
pub fn load_user_path(path: &std::path::Path) -> Result<Presentation> {
    let bytes = crate::io::read_app_file(path).map_err(crate::EngineError::Other)?;
    deckcraft_format::load(&bytes).map_err(|e| crate::EngineError::Other(e.to_string()))
}

/// Load the user template called `name` (case-insensitive, `.deckcraft` optional).
#[cfg(not(target_arch = "wasm32"))]
pub fn load_user(dir: &std::path::Path, name: &str) -> Option<Result<Presentation>> {
    let want = name.trim().trim_end_matches(".deckcraft");
    let path = user_templates(dir).into_iter().find(|t| t.name.eq_ignore_ascii_case(want))?.path;
    Some(load_user_path(&path))
}

/// Save `doc` as a user template called `name` in `dir`, without replacing an existing one:
/// a name that is taken gets " 2", " 3"… A name is never empty and never contains a path
/// separator or a control character.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_user(dir: &std::path::Path, doc: &Presentation, name: &str) -> Result<std::path::PathBuf> {
    let mut base: String = name
        .trim()
        .trim_end_matches(".deckcraft")
        .chars()
        .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '-' } else { c })
        .take(80)
        .collect();
    base = base.trim().trim_matches('.').to_string();
    if base.is_empty() {
        base = "Template".into();
    }
    let bytes = deckcraft_format::save(doc).map_err(|e| crate::EngineError::Other(e.to_string()))?;
    let mut n = 1;
    loop {
        let file = if n == 1 { format!("{base}.deckcraft") } else { format!("{base} {n}.deckcraft") };
        let path = dir.join(file);
        if path.exists() {
            n += 1;
            if n > 999 {
                return Err(bad("file.saveTemplate", "there are too many templates with this name"));
            }
            continue;
        }
        crate::io::write_app_file(&path, &bytes).map_err(crate::EngineError::Other)?;
        return Ok(path);
    }
}

fn text_with_levels(text: &str) -> TextBody {
    let mut body = TextBody::from_text(text);
    for para in &mut body.paragraphs {
        let tabs = para.runs.first().map(|r| r.text.chars().take_while(|c| *c == '\t').count()).unwrap_or(0);
        if tabs > 0
            && let Some(run) = para.runs.first_mut()
        {
            run.text = run.text.trim_start_matches('\t').to_string();
            para.level = tabs.min(8) as u8;
        }
    }
    body
}

/// A new presentation filled with `name`'s slides. `size` is the slide size in points.
pub fn build(name: &str, size: Size) -> Result<Presentation> {
    let starter = find(name).ok_or_else(|| {
        let names = starters().iter().map(|t| t.name).collect::<Vec<_>>().join(", ");
        bad("file.new", format!("no template `{name}`; use {names}"))
    })?;
    let theme = deckcraft_model::theme::builtin_themes()
        .into_iter()
        .find(|t| t.name.eq_ignore_ascii_case(starter.theme))
        .ok_or_else(|| bad("file.new", format!("template `{}` names a missing theme `{}`", starter.name, starter.theme)))?;
    let mut doc = defaults::blank_presentation(size, theme, false);
    for seed in starter.slides {
        let key = Value::String(seed.layout.to_string());
        let layout = find_layout(&doc, &key, 0)
            .ok_or_else(|| bad("file.new", format!("template `{}` uses an unknown layout `{}`", starter.name, seed.layout)))?;
        let mut slide = defaults::new_slide(&mut doc, layout).ok_or_else(|| bad("file.new", crate::cmd::NO_ID_SPACE))?;
        if !seed.title.is_empty()
            && let Some(sh) = slide.shapes.iter_mut().find(|sh| sh.ph_type().is_some_and(|k| k.is_title()))
        {
            sh.text = Some(text_with_levels(seed.title));
        }
        let mut bodies = seed.bodies.iter();
        for sh in slide.shapes.iter_mut().filter(|sh| sh.ph_type().is_some_and(|k| !k.is_title())) {
            let Some(text) = bodies.next() else { break };
            if !text.is_empty() {
                sh.text = Some(text_with_levels(text));
            }
        }
        if bodies.next().is_some() {
            return Err(bad("file.new", format!("template `{}` has more text than the `{}` layout holds", starter.name, seed.layout)));
        }
        if !seed.notes.is_empty() {
            slide.notes = text_with_levels(seed.notes);
        }
        doc.slides.push(Arc::new(slide));
    }
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::Session;

    fn body(doc: &Presentation, slide: usize, n: usize) -> (String, u8) {
        let para_slide = &doc.slides[slide];
        let text = para_slide
            .shapes
            .iter()
            .filter(|sh| sh.ph_type().is_some_and(|k| !k.is_title()))
            .nth(n)
            .and_then(|sh| sh.text.as_ref())
            .cloned()
            .unwrap_or_default();
        let level = text.paragraphs.get(1).map(|p| p.level).unwrap_or(0);
        (text.text(), level)
    }

    #[test]
    fn every_starter_opens_on_its_theme_and_is_a_clean_document() {
        let mut s = Session::new();
        for starter in starters() {
            s.execute("file.new", &json!({"template": starter.name.to_lowercase()})).unwrap();
            let st = s.active().unwrap();
            assert_eq!(st.doc.masters[0].theme.name, starter.theme, "{}", starter.name);
            assert_eq!(st.doc.slides.len(), starter.slides.len(), "{}", starter.name);
            assert_eq!(st.doc.slides[0].title(), starter.slides[0].title, "{}", starter.name);
            assert!(!st.is_dirty(), "{}", starter.name);
            assert!(st.history.undo.is_empty(), "{}", starter.name);
            assert_eq!(st.selection.slide, 0);
        }
        let listed = s.execute("file.templates", &json!({})).unwrap();
        assert_eq!(listed["templates"].as_array().map(Vec::len), Some(starters().len()));
    }

    #[test]
    fn briefing_keeps_notes_columns_and_outline_levels() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"template": "Briefing", "size": [800.0, 450.0]})).unwrap();
        let doc = &s.active().unwrap().doc;
        assert_eq!(doc.slide_size.width, 800.0);
        assert_eq!(doc.slides[0].notes_text(), "Open with what changed since last week.");
        let (agenda, level) = body(doc, 1, 0);
        assert!(agenda.contains("One concrete change"), "{agenda}");
        assert_eq!(level, 1);
        let (done, _) = body(doc, 3, 0);
        let (next, _) = body(doc, 3, 1);
        assert!(done.starts_with("Done"), "{done}");
        assert!(next.starts_with("Next"), "{next}");
    }

    #[test]
    fn pitch_comparison_fills_both_columns() {
        let doc = build("Pitch", defaults::WIDE).unwrap();
        assert_eq!(body(&doc, 3, 0).0, "Today");
        assert!(body(&doc, 3, 1).0.contains("buried"));
        assert_eq!(body(&doc, 3, 2).0, "After");
        assert!(body(&doc, 3, 3).0.contains("top"));
    }

    #[test]
    fn search_matches_name_summary_and_theme() {
        assert_eq!(matching("ridge").iter().map(|t| t.name).collect::<Vec<_>>(), vec!["Field Notes"]);
        assert_eq!(matching("nocturne").iter().map(|t| t.name).collect::<Vec<_>>(), vec!["Evening Talk", "Conference Session"]);
        assert_eq!(matching("").len(), starters().len());
        assert!(matching("facet").is_empty());
    }

    #[test]
    fn every_category_has_starters_and_search_finds_them() {
        let cats = categories();
        assert_eq!(cats.iter().map(|(c, _)| *c).collect::<Vec<_>>(), CATEGORIES.to_vec(), "all ten categories are offered");
        assert!(cats.iter().all(|(_, n)| *n > 0));
        assert_eq!(cats.iter().map(|(_, n)| n).sum::<usize>(), starters().len());
        assert_eq!(by_category("technology").len(), 2);
        assert_eq!(by_category("").len(), starters().len());
        assert_eq!(by_category("all").len(), starters().len());
        assert!(by_category("astronomy").is_empty());
        assert_eq!(matching("postmortem").iter().map(|t| t.name).collect::<Vec<_>>(), vec!["Incident Review"]);
        assert_eq!(matching("portfolio").iter().map(|t| t.name).collect::<Vec<_>>(), vec!["Case Study"]);
    }

    #[test]
    fn every_starter_names_a_known_category_theme_and_has_search_tags() {
        let themes = deckcraft_model::theme::builtin_themes();
        for starter in starters() {
            assert!(CATEGORIES.contains(&starter.category), "{} has category {}", starter.name, starter.category);
            assert!(themes.iter().any(|t| t.name == starter.theme), "{} names theme {}", starter.name, starter.theme);
            assert!(!starter.tags.is_empty(), "{} has no search tags", starter.name);
            assert!(!starter.slides.is_empty(), "{} has no slides", starter.name);
        }
        let names: Vec<&str> = starters().iter().map(|t| t.name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "starter names are unique");
    }

    #[test]
    fn unknown_blank_and_theme_are_refused_but_a_theme_override_is_allowed() {
        let mut s = Session::new();
        let unknown = s.execute("file.new", &json!({"template": "Ion"})).unwrap_err();
        assert!(unknown.to_string().contains("Briefing"), "{unknown}");
        let unknown_theme = s.execute("file.new", &json!({"template": "Lesson", "theme": "No Such Theme"})).unwrap_err();
        assert!(unknown_theme.to_string().contains("no theme"), "{unknown_theme}");
        let blank = s.execute("file.new", &json!({"template": "Lesson", "blank": true})).unwrap_err();
        assert!(blank.to_string().contains("blank"), "{blank}");
        assert!(s.active().is_none());
        s.execute("file.new", &json!({"template": "Lesson", "theme": "Harbor"})).unwrap();
        assert_eq!(s.doc().unwrap().doc.masters[0].theme.name, "Harbor");
    }

    #[test]
    fn a_starter_round_trips_through_pptx() {
        let doc = build("Evening Talk", defaults::WIDE).unwrap();
        let bytes = deckcraft_pptx::export(&doc).unwrap();
        let back = deckcraft_pptx::import(&bytes).unwrap();
        assert_eq!(back.slides.len(), doc.slides.len());
        for (before, after) in doc.slides.iter().zip(back.slides.iter()) {
            assert_eq!(after.title(), before.title());
        }
        assert!(back.slides[3].shapes.iter().any(|sh| sh.text.as_ref().is_some_and(|t| t.text().contains("bedroom"))));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn user_templates_save_list_open_and_never_replace_one() {
        let dir = std::env::temp_dir().join(format!("deckcraft-user-templates-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Session::new();
        s.template_dir = Some(dir.clone());
        s.execute("file.new", &json!({"template": "Lesson"})).unwrap();
        let saved = s.execute("file.saveTemplate", &json!({"name": "My Lesson"})).unwrap();
        assert_eq!(saved["name"], "My Lesson");
        let path = saved["path"].as_str().unwrap().to_string();
        assert!(path.ends_with("My Lesson.deckcraft"), "{path}");

        // The same name again gets " 2", never replacing the first file.
        let again = s.execute("file.saveTemplate", &json!({"name": "My Lesson"})).unwrap();
        assert_eq!(again["name"], "My Lesson 2");
        assert!(std::path::Path::new(&path).is_file());

        // Listed, and openable by name (the case of the name doesn't matter).
        let listed = s.execute("file.templates", &json!({})).unwrap();
        let user: Vec<&Value> = listed["templates"].as_array().unwrap().iter().filter(|t| t["user"] == true).collect();
        assert_eq!(user.len(), 2);
        assert_eq!(user[0]["category"], "Yours");
        assert_eq!(user[0]["slides"], 5);
        assert!(user[0]["theme"].as_str().is_some_and(|t| !t.is_empty()));
        let opened = s.execute("file.new", &json!({"template": "my lesson"})).unwrap();
        assert_eq!(opened["slides"], 5);
        assert_eq!(s.active().unwrap().doc.slides[0].title(), "How a lock works");

        // A name that is all path separators becomes a safe file name, not a path.
        let odd = s.execute("file.saveTemplate", &json!({"name": "../bad/name"})).unwrap();
        let odd = odd["name"].as_str().unwrap();
        assert!(!odd.contains('/') && !odd.contains(".."), "{odd}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_damaged_user_template_is_listed_but_refused_when_opened() {
        let dir = std::env::temp_dir().join(format!("deckcraft-bad-template-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Broken.deckcraft"), b"not a presentation").unwrap();
        let list = user_templates(&dir);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Broken");
        assert_eq!(list[0].slides, 0);
        assert!(load_user(&dir, "Broken").is_some_and(|r| r.is_err()));
        let mut s = Session::new();
        s.template_dir = Some(dir.clone());
        assert!(s.execute("file.new", &json!({"template": "Broken"})).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
