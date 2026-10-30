//! Character page model: the five island edge styles, the seven reaction chips for previews, and the pet picker (registry of
//! built-in pets by group, the "None" card, the section captions, selection). Only ids, groups, texts and the selection logic
//! live here; drawing the previews belongs to the pets / characters engine (`pets.js`, `characters.js` today).
//!
//! The pet table is the registry as `PillPets` builds it after `pets.js` + `pets-chars*.js`, `pets-creatures.js`,
//! `pets-things.js` have loaded (28 ids, three groups). External pets that call `PillPets.add` at runtime are appended by the
//! pets engine through `PetPicker::add`.
use crate::settings::calls::BackendCall;
use crate::settings::tr::{tr, Lang, Tr};
use serde_json::json;

pub struct StyleCard {
    pub id: &'static str,
    pub name: Tr,
    pub desc: Tr,
}

/// Island edge styles (`character`), in card order.
pub const STYLES: [StyleCard; 5] = [
    StyleCard { id: "liquid", name: tr("Жидкий край", "Liquid edge"), desc: tr("Нижний край острова тянется к курсору и красится в цвет события.", "The island edge leans to the cursor and takes the colour of the event.") },
    StyleCard { id: "tail", name: tr("Хвост", "Tail"), desc: tr("В острове кто-то живёт, но видно только хвост.", "Someone lives in the island, but only the tail shows.") },
    StyleCard { id: "legs", name: tr("Ножки", "Legs"), desc: tr("У острова есть глаза и ножки: он переминается под музыку и закрывает глаза во сне.", "The island has eyes and little legs: it steps to the music and shuts its eyes to sleep.") },
    StyleCard { id: "glow", name: tr("Свечение", "Glow"), desc: tr("Под островом горит мягкий свет цвета события.", "A soft light in the colour of the event glows under the island.") },
    StyleCard { id: "none", name: tr("Без украшений", "Plain"), desc: tr("Просто чёрный остров, как раньше.", "Just a black island, like before.") },
];

pub struct Reaction {
    /// Island reaction id (the pet engine calls "ai" "thinking").
    pub id: &'static str,
    pub name: Tr,
    /// Edge line colour of the liquid preview.
    pub line: &'static str,
    /// Glow colour of the glow preview.
    pub glow: &'static str,
}

/// The "Preview a reaction" chips.
pub const REACTIONS: [Reaction; 7] = [
    Reaction { id: "calm", name: tr("Покой", "Calm"), line: "#3a393e", glow: "#ffffff" },
    Reaction { id: "music", name: tr("Музыка", "Music"), line: "#d8836a", glow: "#d8836a" },
    Reaction { id: "ai", name: tr("ИИ думает", "AI thinking"), line: "#9e8fe8", glow: "#9e8fe8" },
    Reaction { id: "done", name: tr("Готово", "Done"), line: "#8fbf4a", glow: "#8fbf4a" },
    Reaction { id: "attention", name: tr("Внимание", "Attention"), line: "#e8a040", glow: "#e8a040" },
    Reaction { id: "file", name: tr("Файл", "File"), line: "#5a78a8", glow: "#5a78a8" },
    Reaction { id: "sleep", name: tr("Сон", "Sleep"), line: "#2c2b2f", glow: "#888888" },
];

pub const REACTION_LABEL: Tr = tr("Посмотреть реакцию:", "Preview a reaction:");

/// The reaction name the pet engine uses ("ai" is "thinking" there).
pub fn pet_state_for(reaction: &str) -> &str {
    if reaction == "ai" {
        "thinking"
    } else {
        reaction
    }
}

pub struct PetGroup {
    pub id: &'static str,
    pub name: Tr,
}

pub const GROUPS: [PetGroup; 3] = [
    PetGroup { id: "characters", name: tr("Персонажи", "Characters") },
    PetGroup { id: "creatures", name: tr("Существа", "Creatures") },
    PetGroup { id: "things", name: tr("Вещи", "Things") },
];

pub struct PetDef {
    pub id: &'static str,
    pub group: &'static str,
    pub name: Tr,
    pub desc: Tr,
}

macro_rules! pets {
    ($(($id:expr, $g:expr, $nr:expr, $ne:expr, $dr:expr, $de:expr)),* $(,)?) => {
        [$(PetDef { id: $id, group: $g, name: tr($nr, $ne), desc: tr($dr, $de) }),*]
    };
}

/// Registry order = card order inside a group.
pub const PETS: [PetDef; 28] = pets![
    ("cat", "characters", "Кот", "Cat", "Сидит, хвост и уши живые", "Sits, lively tail and ears"),
    ("ghost", "characters", "Призрак", "Ghost", "Парит над полом, под ним тень", "Floats above the floor with a tiny shadow"),
    ("bunny", "characters", "Зайка", "Bunny", "Сидит и прыгает", "Sits and hops"),
    ("robot", "characters", "Робот", "Robot", "Стоит с антенной, глаза сканируют", "Stands with its antenna, eyes scan"),
    ("slime", "characters", "Слайм", "Slime", "Сидит и пружинит", "Sits and squishes"),
    ("claude8", "characters", "Клод 8-бит", "Claude 8-bit", "Пиксельный Клод стоит на ножках, кадры шагами, как в 8-битной игре", "Pixel Claude stands on its legs, stepped frames like an 8-bit game"),
    ("pixel", "characters", "Пиксель", "Pixel", "8-битный герой: квадратные глазки, шаркающие ножки, цвет по настроению", "8-bit hero: square eyes, shuffling feet, colour follows the mood"),
    ("spark", "characters", "Искра", "Spark", "Огонёк с лицом: цвет и мина показывают настроение", "A glowing ember face: colour and expression show the mood"),
    ("eyes", "characters", "Глазки", "Eyes", "Только глаза: моргают, смотрят за курсором, щурятся", "Just the eyes: blink, follow the cursor, squint"),
    ("bot", "characters", "Бот", "Bot", "Робот с экраном вместо лица: настроение рисуют светящиеся пиксели", "A robot with a screen for a face: the mood is drawn in glowing pixels"),
    ("kaomoji", "characters", "Каомодзи", "Kaomoji", "Живая текстовая рожица ( · ω · ), мимика под настроение", "A living text face ( · ω · ) that changes expression with the mood"),
    ("swarm", "characters", "Рой", "Swarm", "Стая точек: кольцо, волна, орбита, галочка, «!»", "A flock of dots: ring, wave, orbit, check mark, \"!\""),
    ("hamster", "creatures", "Хомяк в колесе", "Hamster", "Крутит колесо тем резвее, чем занятее остров", "Spins its wheel faster the busier the island is"),
    ("stickman", "creatures", "Человечек", "Stickman", "Тонкая фигурка из штрихов: танцует, думает, прыгает и засыпает сидя", "A thin line figure that dances, ponders, jumps and dozes sitting up"),
    ("koi", "creatures", "Рыбка-кои", "Koi", "Кружит от края до края капсулы, под музыку изгибается", "Cruises from end to end of the capsule and sways to the music"),
    ("snail", "creatures", "Улитка", "Snail", "Неторопливо ползёт по капсуле, а во сне прячется в ракушку", "Crawls unhurried along the capsule and hides in its shell to sleep"),
    ("campfire", "things", "Костёр", "Campfire", "Огонёк с глазками между брёвен: пляшет под музыку, дымит, когда думает", "A little flame with eyes between two logs: dances to music, smokes while thinking"),
    ("lava-lamp", "things", "Лава-лампа", "Lava lamp", "Капли воска всплывают и сливаются, цвет стекла следует за реакцией", "Wax blobs rise and merge, the glass takes the reaction colour"),
    ("lens", "things", "Объектив", "Lens", "Живой зелёный глаз в объективе: диафрагма моргает и трепещет под музыку", "A living green eye in a lens: the iris blinks and flutters to music"),
    ("coffee", "things", "Кофе", "Coffee", "Кружка с личиком: пар танцует под музыку, а в «Готово» складывается в сердечко", "A mug with a little face: steam dances to music and folds into a heart when done"),
    ("newton", "things", "Маятник Ньютона", "Newton's cradle", "Офисная игрушка вместо персонажа: крайние шарики качаются в такт реакции", "An office toy instead of a character: the end balls swing to the mood"),
    ("pulse", "things", "Пульс", "Pulse", "Кардиомонитор: сердце бьётся в такт реакции, линия бежит слева направо", "A heart monitor: the heart beats with the mood, the trace runs left to right"),
    ("kitty", "characters", "Котик", "Kitty", "Мордочка с ушами и усами, колокольчик меняет цвет", "A muzzle with ears and whiskers; the bell changes colour"),
    ("drop", "characters", "Капля", "Drop", "Мягкое желе с румянцем, колышется под музыку", "Soft jelly with blush cheeks; wobbles to the music"),
    ("jelly", "characters", "Медуза", "Jellyfish", "Светится в тёмной воде, пульсирует, щупальца качаются", "Glows in dark water, pulses, tentacles sway"),
    ("sloth", "characters", "Ленивец", "Sloth", "Висит, уцепившись за край, и раскачивается", "Hangs from the edge by its arms and sways"),
    ("astronaut", "characters", "Космонавт", "Astronaut", "Парит в невесомости, цвет стекла шлема по реакции", "Floats weightlessly; the helmet glass takes the reaction colour"),
    ("sprout", "characters", "Росток", "Sprout", "Листья качаются, от радости распускается цветок", "Leaves sway; a flower blooms when it is happy"),
];

pub const NONE_PET_NAME: Tr = tr("Без питомца", "None");
pub const NONE_PET_DESC: Tr = tr("Остров без жителей.", "An island with nobody on it.");
pub const PET_CAPTION: Tr = tr("Питомец", "Pet");

/// A pet card with static texts (the registry as shipped, "None" first).
pub struct PetCard {
    pub id: &'static str,
    pub group: &'static str,
    pub name: Tr,
    pub desc: Tr,
}

pub fn pet_cards() -> Vec<PetCard> {
    let mut v = vec![PetCard { id: "none", group: "characters", name: NONE_PET_NAME, desc: NONE_PET_DESC }];
    v.extend(PETS.iter().map(|p| PetCard { id: p.id, group: p.group, name: p.name, desc: p.desc }));
    v
}

/// A pet added at runtime through `PillPets.add` (ids are `[a-z0-9-]+`, unique; an unknown group falls into "characters").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtraPet {
    pub id: String,
    pub group: String,
    pub name: (String, String),
    pub desc: (String, String),
}

/// A card as the renderer draws it, texts already in the chosen language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardView {
    pub id: String,
    pub name: String,
    pub desc: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub caption: String,
    pub cards: Vec<CardView>,
}

/// The pet picker: sections of cards, selection and the patch a click writes (`pet`).
#[derive(Debug, Default)]
pub struct PetPicker {
    extra: Vec<ExtraPet>,
}

impl PetPicker {
    /// Register an external pet. False when the id is malformed or already taken (the engine logs "rejected").
    pub fn add(&mut self, p: ExtraPet) -> bool {
        let ok = !p.id.is_empty() && p.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') && !self.knows(&p.id);
        if ok {
            self.extra.push(p);
        }
        ok
    }

    pub fn knows(&self, id: &str) -> bool {
        id == "none" || PETS.iter().any(|p| p.id == id) || self.extra.iter().any(|p| p.id == id)
    }

    /// "none" first, then the registry order (the order `pids` has in the JS page).
    pub fn ids(&self) -> Vec<String> {
        std::iter::once("none".to_string()).chain(PETS.iter().map(|p| p.id.to_string())).chain(self.extra.iter().map(|p| p.id.clone())).collect()
    }

    /// One grid per non-empty group. The first caption is "Pet" (plus " · <group>" when more than one group has pets), the
    /// others are the group name; "None" is the first card of the first section.
    pub fn sections(&self, lang: Lang) -> Vec<Section> {
        let card = |id: &str, name: String, desc: String| CardView { id: id.to_string(), name, desc };
        let mut out: Vec<Section> = Vec::new();
        let mut groups: Vec<(&'static PetGroup, Vec<CardView>)> = Vec::new();
        for g in GROUPS.iter() {
            let mut cards: Vec<CardView> = PETS.iter().filter(|p| p.group == g.id).map(|p| card(p.id, p.name.get(lang).to_string(), p.desc.get(lang).to_string())).collect();
            for x in self.extra.iter().filter(|x| (if GROUPS.iter().any(|q| q.id == x.group) { x.group.as_str() } else { "characters" }) == g.id) {
                let (n, d) = if lang == Lang::Ru { (&x.name.0, &x.desc.0) } else { (&x.name.1, &x.desc.1) };
                cards.push(card(&x.id, n.clone(), d.clone()));
            }
            if !cards.is_empty() {
                groups.push((g, cards));
            }
        }
        let many = groups.len() > 1;
        for (gi, (g, mut cards)) in groups.into_iter().enumerate() {
            let caption = if gi == 0 {
                cards.insert(0, card("none", NONE_PET_NAME.get(lang).to_string(), NONE_PET_DESC.get(lang).to_string()));
                format!("{}{}", PET_CAPTION.get(lang), if many { format!(" · {}", g.name.get(lang)) } else { String::new() })
            } else {
                g.name.get(lang).to_string()
            };
            out.push(Section { caption, cards });
        }
        out
    }

    /// A click on a card: `{ pet: id }`. Unknown ids write nothing.
    pub fn select(&self, id: &str) -> Option<BackendCall> {
        self.knows(id).then(|| BackendCall::put("pet", json!(id)))
    }
}

/// A click on an island-style card: `{ character: id }`.
pub fn select_style(id: &str) -> Option<BackendCall> {
    STYLES.iter().any(|s| s.id == id).then(|| BackendCall::put("character", json!(id)))
}

/// The "Preview a reaction" chips: the chosen reaction drives the island previews and every pet preview.
#[derive(Debug)]
pub struct ReactionPreview {
    current: &'static str,
}

impl Default for ReactionPreview {
    fn default() -> Self {
        ReactionPreview { current: "calm" }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewState {
    pub island: &'static str,
    pub pet: &'static str,
}

impl ReactionPreview {
    pub fn current(&self) -> &'static str {
        self.current
    }
    pub fn pet_state(&self) -> &'static str {
        if self.current == "ai" {
            "thinking"
        } else {
            self.current
        }
    }
    /// Pick a chip. None for an unknown id.
    pub fn set(&mut self, id: &str) -> Option<PreviewState> {
        let r = REACTIONS.iter().find(|r| r.id == id)?;
        self.current = r.id;
        Some(PreviewState { island: r.id, pet: self.pet_state() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_consistent() {
        let mut seen = std::collections::HashSet::new();
        for p in PETS.iter() {
            assert!(seen.insert(p.id) && p.id != "none", "{}", p.id);
            assert!(p.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'), "{}", p.id);
            assert!(GROUPS.iter().any(|g| g.id == p.group) && p.name.is_complete() && p.desc.is_complete(), "{}", p.id);
        }
        assert_eq!(PETS.len(), 28);
        assert_eq!(PETS.iter().filter(|p| p.group == "creatures").count(), 4);
        assert_eq!(PETS.iter().filter(|p| p.group == "things").count(), 6);
        assert!(STYLES.iter().all(|s| s.name.is_complete() && s.desc.is_complete()));
        assert_eq!(STYLES.iter().map(|s| s.id).collect::<Vec<_>>(), ["liquid", "tail", "legs", "glow", "none"]);
        assert_eq!(REACTIONS.iter().map(|r| r.id).collect::<Vec<_>>(), ["calm", "music", "ai", "done", "attention", "file", "sleep"]);
    }

    #[test]
    fn sections_follow_the_js_captions() {
        let pk = PetPicker::default();
        let s = pk.sections(Lang::En);
        assert_eq!(s.iter().map(|x| x.caption.as_str()).collect::<Vec<_>>(), ["Pet · Characters", "Creatures", "Things"]);
        assert_eq!((s[0].cards[0].id.as_str(), s[0].cards[0].name.as_str()), ("none", "None"));
        assert_eq!(s[0].cards[1].id, "cat");
        assert_eq!(s.iter().map(|x| x.cards.len()).sum::<usize>(), 29, "28 pets + None");
        assert_eq!(pk.sections(Lang::Ru)[0].caption, "Питомец · Персонажи");
        assert_eq!(pk.ids().len(), 29);
        assert_eq!(pk.ids()[0], "none");
    }

    #[test]
    fn selection_writes_only_known_ids() {
        let mut pk = PetPicker::default();
        assert_eq!(pk.select("koi"), Some(BackendCall::put("pet", json!("koi"))));
        assert_eq!(pk.select("none"), Some(BackendCall::put("pet", json!("none"))));
        assert_eq!(pk.select("dragon"), None);
        let dragon = ExtraPet { id: "dragon".into(), group: "weird".into(), name: ("Дракон".into(), "Dragon".into()), desc: ("".into(), "".into()) };
        assert!(pk.add(dragon.clone()) && !pk.add(dragon), "duplicate ids are rejected");
        assert!(!pk.add(ExtraPet { id: "Bad Id".into(), group: "things".into(), name: Default::default(), desc: Default::default() }));
        assert!(pk.select("dragon").is_some());
        assert!(pk.sections(Lang::En)[0].cards.iter().any(|c| c.id == "dragon"), "an unknown group falls into Characters");
        assert_eq!(select_style("glow"), Some(BackendCall::put("character", json!("glow"))));
        assert_eq!(select_style("rainbow"), None);
    }

    #[test]
    fn reaction_chips_drive_island_and_pets() {
        let mut r = ReactionPreview::default();
        assert_eq!((r.current(), r.pet_state()), ("calm", "calm"));
        assert_eq!(r.set("ai"), Some(PreviewState { island: "ai", pet: "thinking" }));
        assert_eq!(r.set("music"), Some(PreviewState { island: "music", pet: "music" }));
        assert_eq!(r.set("nope"), None);
        assert_eq!(r.current(), "music");
        assert_eq!(pet_state_for("ai"), "thinking");
    }
}
