use demoinfocs2_lite::{CsDemoParserState, event::MapChangeEvent, game_event::derive::GameEvent};
use macro_derive::EntityClass;
use std::io::BufReader;

fn main() -> Result<(), std::io::Error> {
    env_logger::init();

    let file = std::env::args()
        .nth(1)
        .expect("Please provide a demo file path as the first argument");
    let file = std::fs::File::open(file).expect("Failed to open demo file");

    let time = std::time::Instant::now();

    let mut parser =
        demoinfocs2_lite::CsDemoParser::new(BufReader::with_capacity(1024 * 128, file))?;

    parser
        .event_manager
        .register_listener(|event: &MapChangeEvent, _state: &CsDemoParserState| {
            println!("Map changed: {}", event.map_name);

            Ok(())
        });

    parser.register_game_event_serializer_factory("player_hurt", PlayerHurtEvent::factory)?;
    parser.event_manager.register_listener(
        |event: &PlayerHurtEvent, _state: &CsDemoParserState| {
            // println!("{event:?}");

            Ok(())
        },
    );
    parser.register_entity_serializer("CCSPlayerController", CCSPlayerController::new_serializer);
    parser.register_entity_serializer("CCSGameRulesProxy", CCSGameRulesProxy::new_serializer);
    parser.register_entity_serializer("CCSGameRules", CCSGameRules::new_serializer);

    loop {
        if !parser.read_frame()? {
            break;
        }
    }

    println!("time: {:?}", time.elapsed());

    Ok(())
}

#[derive(GameEvent, Default, Debug)]
#[game_event(crate_path = demoinfocs2_lite)]
pub struct PlayerHurtEvent {
    pub dmg_health: u16,
    pub dmg_armor: u8,
    pub health: u8,
    pub armor: u8,
    pub weapon: String,
}

#[derive(EntityClass, Clone, Default)]
#[entity(crate_path = demoinfocs2_lite)]
pub struct CCSPlayerController {
    #[entity(name = "m_iPing")]
    pub ping: u64,
    #[entity(name = "m_iszPlayerName", on_changed = Self::on_changed)]
    pub player_name: String,
}

#[derive(EntityClass, Clone, Default)]
#[entity(crate_path = demoinfocs2_lite)]
pub struct CCSGameRulesProxy {
    #[entity(name = "m_pGameRules")]
    pub game_rules: Option<CCSGameRules>,
}

#[derive(EntityClass, Clone, Default)]
#[entity(crate_path = demoinfocs2_lite)]
pub struct CCSGameRules {
    #[entity(name = "m_pGameModeRules")]
    pub game_mode_rules: usize,
}

impl CCSPlayerController {
    fn on_changed(&mut self) -> Result<(), std::io::Error> {
        println!("Name changed: {} (ping {}ms)", self.player_name, self.ping);
        Ok(())
    }
}
