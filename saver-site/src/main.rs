#![feature(proc_macro_hygiene, decl_macro)]
use std::sync::Mutex;
use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;

use serde::Serialize;
use rocket_contrib::templates::Template;
use rocket_contrib::serve::StaticFiles;
use rusqlite::{params, Connection};

#[macro_use]
extern crate lazy_static;
#[macro_use] 
extern crate rocket;

#[get("/")]
fn index() -> Template {
    Template::render("home", ())
}

#[derive(Serialize)]
struct MissingConvo(i64);

#[get("/convo?<conversation_id>")]
fn conversation(conversation_id: i64) -> Template {
    let conn = CONN.lock().unwrap();

    let mut stmt = conn.prepare("SELECT MessageIds, ServerId, Tags FROM Conversations WHERE ConversationId=?1").unwrap();
    let mut conversation = stmt.query(params![conversation_id]).unwrap();
    let conversation = match conversation.next().unwrap(){
        Some(conv) => conv,
        None => return Template::render("missing_convo", MissingConvo(conversation_id))
    };
    
    let message_ids = conversation.get_unwrap::<usize, String>(0);
    let message_ids = {
        let mut split = message_ids.split(",").collect::<Vec<_>>();
        split.pop();
        split.into_iter()
    };

    let server_id = conversation.get_unwrap::<usize, i64>(1);
    //Tags for the conversation. Added by sv_grab x tag1 tag2 tag3 tag4
    let tags = conversation.get_unwrap::<usize, Option<String>>(2).unwrap_or(String::from(""));
    let tags = if tags == "" {
        vec![].into_iter()
    } else {
        tags.split(',').collect::<Vec<_>>().into_iter()
    };

    let mut server = conn.prepare("SELECT Name,Tags FROM Servers WHERE ServerId=?1").unwrap();
    let mut server = server.query(params![server_id]).unwrap();
    let server = server.next().unwrap().unwrap();

    let server_name = server.get_unwrap::<usize, String>(0);
    //Tags for the server. Added by the admin at join
    let server_tags = server.get_unwrap::<usize, Option<String>>(1).unwrap_or(String::from(""));
    let server_tags = server_tags.split(',');

    let mut anon_count = 0;

    let mut messages = message_ids.map(|message_id| {
        let mut hasher = DefaultHasher::new();

        let mut msg = conn.prepare("SELECT AuthorName, Content, Timestamp, AuthorIsBot FROM Messages WHERE MessageId=?1 AND ServerId=?2").unwrap();
        let mut msg = msg.query(params![message_id, server_id]).unwrap();
        let msg = msg.next().unwrap().unwrap();

        //If the name doesn't exist, call it Discord User #{}
        let name = msg.get_unwrap::<usize, Option<String>>(0);
        let name = if let None = name {
            anon_count += 1;
            format!("Discord User #{}", anon_count).to_string()
        } else {name.unwrap()};

        let content = msg.get_unwrap(1);
        let timestamp = msg.get_unwrap::<usize, i64>(2) as u64;
        let is_bot = msg.get_unwrap::<usize, i8>(3) > 0;
        
        hasher.write(name.clone().into_bytes().as_slice());

        let hash = hasher.finish();
        
        let color = format!("{:x}", hasher.finish())[..6].to_string();

        DiscordMessage {
            name,
            bot: is_bot,
            content,
            timestamp,
            color,
            hue_shift: (hash>>hash%16) as u8,
        }
    }).collect::<Vec<_>>();
    
    //The messages are stored in reverse order for the sake of simplicity
    let messages = {
            messages.reverse(); 
            messages
        };


    let convo = Convo {
        server_name,
        messages,
        tags: tags.chain(server_tags).map(|s| s.to_string()).collect()
    };
    

    Template::render("chat_log", convo)
}

#[derive(Serialize)]
struct Convo {
    server_name: String,
    messages:  Vec<DiscordMessage>,
    tags: Vec<String>
}


lazy_static! {
    static ref CONN: Mutex<Connection> = {
        std::env::set_current_dir(std::env::var("SAVER_DIR").unwrap()).unwrap();
        Mutex::new(Connection::open("./conversations.db").unwrap())
    };
}

fn main() {
    dotenv::dotenv().ok();
    rocket::ignite()
            .mount("/", routes![index, conversation])
            .mount("/static", StaticFiles::from("./files"))
            .attach(Template::fairing())
            .launch();
    
}


#[derive(Debug, Serialize)]
struct DiscordMessage {
    name: String,
    bot: bool,
    content: String,
    timestamp: u64,
    color: String,
    hue_shift: u8,
}
