use std::sync::Mutex;

use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, Transaction};
use serenity::async_trait;
use serenity::client::{Client, Context, EventHandler};
use serenity::framework::standard::{
    macros::{command, group},
    Args, CommandResult, StandardFramework,
};
use serenity::model::channel::{Message, ReactionType};
use serenity::model::gateway::Ready;
use serenity::model::guild::{Guild, GuildStatus, Member};
use serenity::model::id::GuildId;
use serenity::model::invite::Invite;

#[macro_use]
extern crate lazy_static;

lazy_static! {
    static ref CONN: Mutex<Connection> = Mutex::new(Connection::open("conversations.db").unwrap());
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();

    //set up discord bot framework
    let framework = StandardFramework::new()
        .configure(|c| c.prefix("sv_"))
        .group(&GENERAL_GROUP);

    let token = std::env::var("DISCORD_TOKEN").expect("Expected a value in $DISCORD_TOKEN");

    let mut client = Client::new(token)
        .event_handler(Handler)
        .framework(framework)
        .await?;

    client.start().await?;

    Ok(())
}

struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn guild_create(&self, ctx: Context, guild: Guild, is_new: bool) {
        if is_new {
            register_server(guild.id, Some(guild.name), None).unwrap();
        }

        for (_, member) in guild.members.iter() {
            register_user(&ctx, &member, &guild.id).await.unwrap();
        }
    }

    async fn ready(&self, ctx: Context, data_about_bot: Ready) {
        for server in data_about_bot
            .guilds
            .iter()
            .filter(|guild_status| match guild_status {
                GuildStatus::OnlineGuild(_) => true,
                _ => false,
            })
        {
            register_server(
                server.id(),
                Some(server.id().to_partial_guild(&ctx.http).await.unwrap().name),
                None,
            )
            .unwrap();
        }
    }

    async fn guild_member_addition(&self, ctx: Context, guild_id: GuildId, member: Member) {
        register_user(&ctx, &member, &guild_id).await.unwrap();
    }
}

async fn register_user(ctx: &Context, member: &Member, guild_id: &GuildId) -> Result<()> {
    let new_user = {
        let conn = CONN.lock().unwrap();

        let id = (*member.user.id.as_u64()) as i64;

        //Bot users are automatically opt-in
        if member.user.bot {
            conn.execute(
                "INSERT OR IGNORE INTO Users (UserId, OptIn) VALUES (?1, 1)",
                params![id],
            )?;
            return Ok(());
        }

        conn.execute(
            "INSERT OR IGNORE INTO Users (UserId) VALUES (?1)",
            params![id],
        )?;
        let mut user_lookup = conn.prepare("SELECT OptIn FROM Users WHERE UserId=?1")?;
        let mut user_lookup = user_lookup.query(params![id])?;
        let user_lookup = user_lookup.next()?;

        user_lookup.is_none()
    };

    let guild_name = guild_id.to_partial_guild(&ctx.http).await?.name;

    if new_user {
        member.user.dm(&ctx.http, |m|{
                m.content(format!("This is a message to let you know that the server you just joined, {}, uses ripcord.rs. \
                            Ripcord is used as follows: \
                            1) Some users have a conversation.\
                            2) Someone decides that the conversation is important/helpful/funny, so they save it using `sv_grab`.\
                            3) These messages get uploaded and shared publicly to ripcord.rs", guild_name))
                 .content("If this is cool with you, react with 👍 to opt in. If it isn't, react with 👎 to opt out. If you're cool with them using your messages, but not your name, react with 🤫")
                 .reactions(['👍', '👎', '🤫'].iter().map(|e| *e))
            }).await?;
    }
    Ok(())
}

fn register_server(id: GuildId, name: Option<String>, invite: Option<Invite>) -> Result<()> {
    let id = *id.as_u64() as i64;
    let link = invite.and_then(|inv| Some(inv.url()));
    CONN.lock().unwrap().execute(
        "INSERT OR IGNORE INTO Servers (ServerId, Name, Link) VALUES (?1, ?2, ?3)",
        params![id, name, link],
    )?;
    Ok(())
}

#[group]
#[commands(grab)]
struct General;

#[command]
async fn grab(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    let channel = msg.channel_id;
    let guild_id = msg.guild_id.ok_or(anyhow!(""))?;

    register_server(
        guild_id,
        Some(guild_id.to_partial_guild(&ctx.http).await?.name),
        None,
    )?;

    let guild_id = *guild_id.as_u64() as i64;

    //check if there's a valid argument
    let number = match args.single::<u8>() {
        Ok(number) => number,
        Err(_) => {
            channel
                .say(
                    &ctx.http,
                    "Invalid argument: expected positive number under 255",
                )
                .await?;
            return Ok(());
        }
    };

    //Get the last [number] messages before the command
    let messages = match channel
        .messages(&ctx.http, |ret| ret.before(msg).limit(number.into()))
        .await
    {
        Ok(msgs) => msgs,
        Err(_) => {
            channel
                .say(
                    &ctx.http,
                    format!("There are less than {} messages in this channel!", number),
                )
                .await?;
            return Ok(());
        }
    };

    let tags = args.rest().replace(" ", ",");

    //We have to wrap this in a function so that if an error gets handled, the compiler is certain
    //that it won't run again (thus causing issues with the mutex)
    if let Err(e) = (|| -> Result<()> {
        let mut conn = CONN.lock().unwrap();
        let msg_transaction = conn.transaction()?;

        let mut message_string = String::from("");

        for message in messages.iter() {
            let user_id = (*message.author.id.as_u64()) as i64;

            (*msg_transaction).execute(
                "INSERT OR IGNORE INTO Users
                                            (UserId)
                                            VALUES (?1)",
                params![user_id],
            )?;
        }

        for message in messages.iter() {
            let id: i64 = (*message.id.as_u64()) as i64;
            let is_bot = message.author.bot;
            let name = &message.author.name;
            let timestamp = message.timestamp.timestamp_millis();
            let content = &message.content;
            let user_id = (*message.author.id.as_u64()) as i64;

            let mut user = (*msg_transaction)
                .prepare("SELECT OptIn FROM Users WHERE UserId=?1")
                .unwrap();
            let mut user = user.query(params![user_id]).unwrap();
            let user = user.next().unwrap().unwrap();

            if let Some(opt) = user.get_unwrap::<usize, Option<i64>>(0) {
                if opt != 0 {
                    return Err(anyhow!(
                        "At least one user in this conversation has opted out."
                    ));
                }
            }

            //Push message ids as a comma-delimited string
            message_string.push_str(&format!("{},", id));

            (*msg_transaction).execute("INSERT OR IGNORE INTO Messages 
                                            (MessageId, AuthorName, Content, Timestamp, AuthorIsBot, ServerId)
                                            VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![id, name, content, timestamp, is_bot, guild_id])?;
        }

        msg_transaction.commit()?;

        let conv_transaction = conn.transaction()?;
        println!("{}", tags);
        if tags == "" {
            (*conv_transaction).execute(
                "INSERT INTO Conversations (MessageIds, ServerId)
                      Values (?1, ?2)",
                params![message_string, guild_id],
            )?;
        } else {
            (*conv_transaction).execute(
                "INSERT INTO Conversations (MessageIds, ServerId, Tags)
                      Values (?1, ?2, ?3)",
                params![message_string, guild_id, tags],
            )?;
        }
        conv_transaction.commit()?;
        Ok(())
    })() {
        println!("{}", e);
        if let Ok(err_msg) = e.downcast::<&str>() {
            msg.channel_id
                .say(&ctx.http, format!("Failed to gather messages: {}", err_msg))
                .await?;
        } else {
            msg.channel_id
                .say(&ctx.http, "Failed to gather messages.")
                .await?;
        }
        return Ok(());
    }

    channel
        .say(&ctx.http, format!("Saved the last {} messages!", number))
        .await?;
    Ok(())
}
