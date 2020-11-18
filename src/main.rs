#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate anyhow;
#[macro_use]
extern crate lazy_static;

#[macro_use]
extern crate log;

mod config;
pub mod entries;
mod freedesktop;
mod style;
mod subscriptions;

use iced::{
    scrollable, text_input, window, Align, Application, Color, Column, Command, Container, Element,
    Length, Row, Scrollable, Settings, Subscription, Text, TextInput,
};
use style::theme::Theme;

use crate::config::OnagreSettings;
use crate::entries::Entries;
use crate::entries::{EntriesState, Entry};
use fuzzy_matcher::skim::SkimMatcherV2;
use iced_native::Event;
use serde::export::Formatter;
use std::collections::HashMap;
use std::process::exit;
use subscriptions::custom::ExternalCommandSubscription;
use subscriptions::desktop_entries::DesktopEntryWalker;

lazy_static! {
    static ref THEME: Theme = Theme::load();
    pub static ref SETTINGS: OnagreSettings = OnagreSettings::get().unwrap_or_default();
}

fn main() -> iced::Result {
    env_logger::init();
    debug!("Starting Onagre in debug mode");

    Onagre::run(Settings {
        window: window::Settings {
            transparent: true,
            ..Default::default()
        },
        default_text_size: 20,
        antialiasing: true,
        ..Default::default()
    })
}

#[derive(Debug)]
struct Onagre {
    modes: Vec<Mode>,
    state: State,
    matcher: OnagreMatcher,
}

#[derive(Debug)]
struct State {
    loading: bool,
    mode_button_idx: usize,
    selected: usize,
    entries: EntriesState,
    scroll: scrollable::State,
    input: text_input::State,
    input_value: String,
}

struct OnagreMatcher {
    matcher: SkimMatcherV2,
}

impl std::fmt::Debug for OnagreMatcher {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "SkimMatcherV2")
    }
}

impl Default for State {
    fn default() -> Self {
        State {
            loading: true,
            mode_button_idx: 0,
            selected: 0,
            entries: EntriesState::default(),
            scroll: Default::default(),
            input: Default::default(),
            input_value: "".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    InputChanged(String),
    DesktopEntryEvent(Entry),
    CustomModeEvent(Vec<Entry>),
    EventOccurred(iced_native::Event),
    Loaded(HashMap<Mode, Vec<Entry>>),
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum Mode {
    Drun,
    Custom(String),
}

impl Application for Onagre {
    type Executor = iced::executor::Default;
    type Message = Message;
    type Flags = ();

    fn new(_flags: Self::Flags) -> (Self, Command<Self::Message>) {
        Onagre::sway_preloads();

        let mut modes = vec![Mode::Drun];

        let custom_modes = SETTINGS
            .modes
            .keys()
            .map(|mode| mode.to_owned())
            .map(Mode::Custom);

        modes.extend(custom_modes);

        (
            Onagre {
                modes: modes.clone(),
                state: State::default(),
                matcher: OnagreMatcher {
                    matcher: SkimMatcherV2::default().ignore_case(),
                },
            },
            Command::perform(entries::cache::get_cached_entries(modes), Message::Loaded),
        )
    }

    fn title(&self) -> String {
        "Onagre".to_string()
    }

    fn background_color(&self) -> Color {
        Color::TRANSPARENT
    }

    fn update(&mut self, message: Self::Message) -> Command<Self::Message> {
        self.state.input.focus(true);

        match message {
            Message::CustomModeEvent(new_entries) => {
                let current_mode = self.get_current_mode().clone();
                let entries = self.state
                    .entries
                    .mode_entries
                    .get_mut(&current_mode)
                    .unwrap();

                let new_entries_filtered: Vec<Entry> = new_entries.into_iter()
                    .filter(|entry| {
                        entries
                            .iter()
                            .find(|current_entry| {
                                current_entry.display_name == entry.display_name
                            })
                            .is_none()
                    })
                    .collect();

                    entries.extend(new_entries_filtered);
                println!("{}", n);
                Command::none()
            }
            Message::InputChanged(input) => {
                self.state.input_value = input;
                self.reset_matches();
                Command::none()
            }
            Message::EventOccurred(event) => {
                self.handle_input(event);
                Command::none()
            }
            Message::DesktopEntryEvent(entry) => {
                let entries = self
                    .state
                    .entries
                    .mode_entries
                    .get_mut(&Mode::Drun)
                    .unwrap();

                let entry_is_known_already = entries
                    .iter()
                    .find(|current_entry| current_entry.display_name == entry.display_name);

                if entry_is_known_already.is_none() {
                    entries.push(entry);
                    // TODO: maybe just match this entry against current
                    // input value and update only if we have a good score
                    self.reset_matches();
                }

                Command::none()
            }
            Message::Loaded(entries) => {
                self.state.entries.mode_entries = entries;
                self.reset_matches();
                Command::none()
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let event = iced_native::subscription::events().map(Message::EventOccurred);
        let desktop_entries = DesktopEntryWalker::subscription().map(Message::DesktopEntryEvent);

        let mut subscriptions = vec![event, desktop_entries];
        if let Mode::Custom(name) = self.get_current_mode() {
            let command = &SETTINGS.modes.get(name).unwrap().source;
            subscriptions.push(
                ExternalCommandSubscription::subscription(command).map(Message::CustomModeEvent),
            );
        }

        Subscription::batch(subscriptions)
    }

    fn view(&mut self) -> Element<'_, Self::Message> {
        let mode_buttons: Row<Message> =
            Self::build_mode_menu(self.state.mode_button_idx, &self.modes);

        let current_mode = self.get_current_mode();
        let matches = self.state.entries.mode_matches.get(current_mode);

        // Build rows from current mode search entries
        let entries_column = if let Some(matches) = matches {
            let rows: Vec<Element<Message>> = matches
                .iter()
                .enumerate()
                .map(|(idx, entry)| {
                    if idx == self.state.selected {
                        self.entry_by_idx(*entry).to_row_selected().into()
                    } else {
                        self.entry_by_idx(*entry).to_row().into()
                    }
                })
                .collect();

            Column::with_children(rows)
        } else {
            Column::new()
        };

        // Scrollable element containing the rows
        let scrollable = Container::new(
            Scrollable::new(&mut self.state.scroll)
                .with_content(entries_column)
                .height(THEME.scrollable.height.into())
                .width(THEME.scrollable.width.into())
                .scrollbar_width(THEME.scrollable.scroller_width)
                .scroller_width(THEME.scrollable.scrollbar_width)
                .style(&THEME.scrollable),
        )
        .style(&THEME.scrollable)
        .padding(THEME.scrollable.padding);

        // Switch mode menu
        let mode_menu = Container::new(
            Row::new()
                .push(mode_buttons)
                .height(THEME.menu.width.into())
                .width(THEME.menu.height.into()),
        )
        .padding(THEME.menu.padding)
        .style(&THEME.menu);

        let search_input = TextInput::new(
            &mut self.state.input,
            "Search",
            &self.state.input_value,
            Message::InputChanged,
        )
        .width(THEME.search.bar.text_width.into())
        .style(&THEME.search.bar);

        let search_bar = Container::new(
            Row::new()
                .spacing(20)
                .align_items(Align::Center)
                .padding(2)
                .push(search_input)
                .width(THEME.search.width.into())
                .height(THEME.search.height.into()),
        )
        .padding(THEME.search.padding)
        .style(&THEME.search);

        let app_container = Container::new(
            Column::new()
                .push(mode_menu)
                .push(search_bar)
                .push(scrollable)
                .align_items(Align::Start)
                .height(Length::Fill)
                .width(Length::Fill)
                .padding(20),
        )
        .style(THEME.as_ref());

        app_container.into()
    }
}

impl Onagre {
    fn entry_by_idx(&self, idx: usize) -> &Entry {
        let mode = self.get_current_mode();
        self.state
            .entries
            .mode_entries
            .get(mode)
            .unwrap()
            .get(idx)
            .unwrap()
    }

    fn entry_mut_by_idx(&mut self, idx: usize) -> &mut Entry {
        let mode = self.get_current_mode().clone();
        self.state
            .entries
            .mode_entries
            .get_mut(&mode)
            .unwrap()
            .get_mut(idx)
            .unwrap()
    }

    fn build_mode_menu(mode_idx: usize, modes: &[Mode]) -> Row<'_, Message> {
        let rows: Vec<Element<Message>> = modes
            .iter()
            .enumerate()
            .map(|(idx, mode)| {
                if idx == mode_idx {
                    Container::new(Text::new(mode.to_string()))
                        .style(&THEME.menu.lines.selected)
                        .width(THEME.menu.lines.selected.width.into())
                        .height(THEME.menu.lines.selected.height.into())
                        .padding(THEME.menu.lines.selected.padding)
                        .into()
                } else {
                    Container::new(Text::new(mode.to_string()))
                        .style(&THEME.menu.lines.default)
                        .width(THEME.menu.lines.default.width.into())
                        .height(THEME.menu.lines.default.height.into())
                        .padding(THEME.menu.lines.default.padding)
                        .into()
                }
            })
            .collect();

        Row::with_children(rows)
    }

    fn run_command(&mut self) -> Command<Message> {
        let mode = self.get_current_mode().clone();
        let selected = self.state.selected;

        let mode_entries = self.state.entries.mode_matches.get(&mode).unwrap();

        let current_entry_idx = *mode_entries.get(selected).unwrap();

        let mut current_entry = self.entry_mut_by_idx(current_entry_idx);

        // This is the single mutable operation we have to do for entry
        current_entry.weight += 1;

        match mode {
            Mode::Drun => {
                let options = current_entry.options.as_ref().unwrap();
                let argv = shell_words::split(&options.exec);

                let args = argv.unwrap();
                let args = args
                    .iter()
                    // Filtering out special freedesktop syntax
                    .filter(|entry| !entry.starts_with('%'))
                    .collect::<Vec<&String>>();

                std::process::Command::new(&args[0])
                    .args(&args[1..])
                    .spawn()
                    .expect("Command failure");
            }
            Mode::Custom(mode_name) => {
                let command = &SETTINGS.modes.get(&mode_name).unwrap().target;
                let command = command.replace("%", &current_entry.display_name);
                let args = shell_words::split(&command).unwrap();
                let args = args.iter().collect::<Vec<&String>>();

                std::process::Command::new(&args[0])
                    .args(&args[1..])
                    .spawn()
                    .expect("Command failure");
            }
        };

        self.flush_all();

        // Is this ok with iced or shall we exit with and internal command ?
        exit(0);
    }

    fn handle_input(&mut self, event: iced_native::Event) {
        use iced_native::keyboard::KeyCode;

        if let Event::Keyboard(keyboard_event) = event {
            if let iced_native::keyboard::Event::KeyPressed { key_code, .. } = keyboard_event {
                match key_code {
                    KeyCode::Up => {
                        if self.state.selected != 0 {
                            self.state.selected -= 1
                        }
                    }
                    KeyCode::Down => {
                        let mode = self.get_current_mode();

                        let max_idx = self.state.entries.mode_matches.get(mode).unwrap().len();

                        if max_idx != 0 && self.state.selected < max_idx - 1 {
                            self.state.selected += 1
                        }
                    }
                    KeyCode::Enter => {
                        self.run_command();
                    }
                    KeyCode::Tab => {
                        self.cycle_mode();
                    }
                    KeyCode::Escape => {
                        self.flush_all();
                        exit(0);
                    }
                    _ => {}
                }
            }
        }
    }

    fn reset_matches(&mut self) {
        self.state.selected = 0;

        let mode = self.get_current_mode().clone();
        if self.state.input_value == "" {
            let matches = self
                .state
                .entries
                .mode_entries
                .get(&mode)
                .unwrap()
                .default_matches();

            self.set_custom_matches(mode, matches);
        } else {
            let matches = self
                .state
                .entries
                .mode_entries
                .get(&mode)
                .unwrap()
                .get_matches(&self.state.input_value, &self.matcher.matcher);

            self.set_custom_matches(mode, matches)
        }
    }

    fn cycle_mode(&mut self) {
        println!("{}/{}", self.state.mode_button_idx, self.modes.len());
        if self.state.mode_button_idx == self.modes.len() - 1 {
            debug!("Changing mode {} -> 0", self.state.mode_button_idx);
            self.state.mode_button_idx = 0
        } else {
            debug!(
                "Changing mode {} -> {}",
                self.state.mode_button_idx,
                self.state.mode_button_idx + 1
            );
            self.state.mode_button_idx += 1
        }
    }

    fn get_current_mode(&self) -> &Mode {
        // Safe unwrap, we control the idx here
        let mode = self.modes.get(self.state.mode_button_idx).unwrap();
        mode
    }

    fn set_custom_matches(&mut self, mode: Mode, matches: Vec<usize>) {
        self.state.entries.mode_matches.insert(mode, matches);
    }

    fn flush_all(&mut self) {
        // This is really dirty but for now the only solution I see to not take the exclusive lock
        self.state
            .entries
            .mode_entries
            .iter()
            .for_each(|(mode, entries)| {
                let mut entries = entries.clone();
                entries.sort_unstable_by(|entry, other| other.weight.cmp(&entry.weight));
                entries::cache::flush_mode_cache(mode, &entries);
            });
    }
}

impl ToString for Mode {
    fn to_string(&self) -> String {
        match &self {
            Mode::Drun => "Drun".to_string(),
            Mode::Custom(name) => name.clone(),
        }
    }
}

impl Onagre {
    fn sway_preloads() {
        // Tell sway to enable floating mode for Onagre
        std::process::Command::new("swaymsg")
            .arg("for_window [app_id=\"Onagre\"] floating enable")
            .output()
            .expect("not on sway");

        // [set|plus|minus] <value>
        // Tells sway to focus on startup
        std::process::Command::new("swaymsg")
            .arg("[app_id=\"Onagre\"] focus")
            .output()
            .expect("not on sway");

        // Tells sway to remove borders on startup
        std::process::Command::new("swaymsg")
            .arg("for_window [app_id=\"Onagre\"] border none ")
            .output()
            .expect("not on sway");

        // Tells sway to remove borders on startup
        std::process::Command::new("swaymsg")
            .arg("for_window [app_id=\"Onagre\"] resize set width 45 ppt height  35 ppt")
            .output()
            .expect("not on sway");
    }
}
