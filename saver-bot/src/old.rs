#[macro_use]
extern crate lazy_static;

use std::env;
use std::sync::{Arc, Mutex};

use chrono::prelude::*;
use rusqlite::types::Value::Null;
use rusqlite::{params, Connection, Transaction};

use serenity::async_trait;
use serenity::client::{Client, Context, EventHandler};
use serenity::framework::standard::{
    macros::{command, group},
    Args, CommandResult, StandardFramework,
};
use serenity::model::channel::Message;
use serenity::model::guild::Guild;

use anyhow::Result;

lazy_static! {
    static ref CONNECTION: Arc<Mutex<Connection>> =
        Arc::new(Mutex::new(Connection::open("conversations.db").unwrap()));
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();

    let framework = StandardFramework::new()
        .configure(|c| c.prefix("sv_"))
        .group(&GENERAL_GROUP);

    let token = env::var("DISCORD_TOKEN").expect("Expected a value in $DISCORD_TOKEN");

    let mut client = Client::new(token)
        .event_handler(Handler)
        .framework(framework)
        .await?;

    client.start().await?;

    Ok(())
}

#[group]
#[commands(grab)]
struct General;

struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn guild_create(&self, _ctx: Context, guild: Guild, is_new: bool) {
        if is_new {
            register_server(*guild.id.as_u64(), Some(guild.name)).unwrap();
        }
    }
}

fn register_server(guild_id: u64, server_name: Option<String>) -> Result<()> {
    let guild_id = guild_id as i64;

    CONNECTION.lock().unwrap().execute(
        "INSERT OR IGNORE INTO Servers (ServerId, Name, Link)
                                              VALUES (?1, ?2, ?3)",
        params![guild_id, server_name, Null],
    )?;
    Ok(())
}

#[command]
async fn grab(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    let server_id = match msg.guild_id {
        Some(id) => *id.as_u64(),
        None => return Ok(()),
    };

    register_server(
        server_id,
        Some(
            msg.guild_id
                .unwrap()
                .to_partial_guild(&ctx.http)
                .await?
                .name,
        ),
    )
    .unwrap();

    let server_id = server_id as i64;

    let number = match args.single::<u8>() {
        Ok(number) => number,
        Err(_) => {
            msg.channel_id
                .say(
                    &ctx.http,
                    "Invalid arguments! Expected positive number under 255",
                )
                .await?;
            return Ok(());
        }
    };

    let messages = match msg
        .channel_id
        .messages(&ctx.http, |retriever| {
            retriever.before(msg).limit(number.into())
        })
        .await
    {
        Ok(msgs) => msgs,
        Err(_) => {
            msg.channel_id
                .say(
                    &ctx.http,
                    format!("There are less than {} messages in this channel!", number),
                )
                .await?;
            return Ok(());
        }
    };

    let mut message_string = String::from("");
    if let Err(e) = (|| -> Result<()> {
        let mut conn = CONNECTION.lock().unwrap();
        let msg_transaction = conn.transaction()?;

        for message in messages.iter() {
                let id: i64 = (*message.id.as_u64()) as i64;
                let is_bot = message.author.bot;
                let name =  &message.author.name;
                let timestamp = message.timestamp.timestamp_millis();
                let content = &message.content;
                
                //Push message ids as a comma-delimited string
                message_string.push_str(&format!("{},", id));
                
                CONNECTION.lock().unwrap().execute("INSERT OR IGNORE INTO Messages (MessageId, AuthorName, Content, Timestamp, AuthorIsBot, ServerId)
                                    VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![id, name, content, timestamp, is_bot, server_id])?;
        }

        msg_transaction.commit()?;
        Ok(())
    })() {
        println!("{}", e);
        msg.channel_id.say(&ctx.http, "Failed to gather messages.").await?;
        return Ok(());
    }
    CONNECTION.lock().unwrap().execute(
        "INSERT INTO Conversations (MessageIds, ServerId)
                      VALUES (?1, ?2)",
        params![message_string, server_id],
    ).unwrap();


    msg.channel_id
        .say(&ctx.http, format!("Saved the last {} messages!", number))
        .await?;
    Ok(())
}

#[derive(Debug)]
struct DiscordMessage {
    name: String,
    bot: bool,
    content: String,
    timestamp: DateTime<chrono::Utc>,
}
