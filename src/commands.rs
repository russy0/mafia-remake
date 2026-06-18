// 역할: poise 슬래시 명령어, 컴포넌트 핸들러, 메시지 이벤트 처리,
//        익명 메시지 릴레이, 통계/리더보드, 역할 정보 조회

#![allow(unused_imports, clippy::too_many_arguments, clippy::collapsible_if)]

use super::{
    Context, Data, Error, RunningGame, Recruitment, ContractorContractDraft,
    AnonymousNameMode, LeaderboardMetric,
    RECRUITMENT_SECONDS, MAX_GAME_PLAYERS,
};
use crate::channel::*;
use crate::embed::*;
use crate::runner::game_loop;
use ab_glyph::{
    Font, FontArc, GlyphId, OutlinedGlyph, PxScale, Rect as GlyphRect, ScaleFont, point,
};
use anyhow::{Context as AnyhowContext, Result, bail};
use dashmap::DashMap;
use image::{ImageFormat, Rgb, RgbImage};
use mafia_remake::config;
use mafia_remake::game::{GameCounts, MafiaGame};
use mafia_remake::model::{
    CITIZEN_SPECIAL_ROLES, CONTRACTOR_GUESS_ROLES, MAFIA_SPECIAL_ROLES, NEUTRAL_SPECIAL_ROLES,
    NightResult, PUBLIC_CITIZEN_SPECIAL_ROLES, PUBLIC_CULT_SPECIAL_ROLES,
    PUBLIC_MAFIA_SPECIAL_ROLES, PUBLIC_NEUTRAL_SPECIAL_ROLES, Phase, Player, Role, VoteResult,
    Winner,
};
use mafia_remake::stats;
use poise::serenity_prelude as serenity;
use poise::serenity_prelude::Mentionable;
use rand::seq::{IndexedRandom, SliceRandom};
use secrecy::ExposeSecret;
use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Notify, RwLock};

#[poise::command(
    slash_command,
    rename = "마피아시작",
    description_localized("ko", "저장된 설정대로 마피아 게임 참가자를 모집하고 시작합니다.")
)]
pub async fn start_game(ctx: Context<'_>) -> Result<(), Error> {
    let Some(guild_id) = ctx.guild_id() else {
        reply_embed(
            ctx,
            "서버 채널에서만 사용할 수 있습니다.",
            "마피아 게임",
            serenity::Colour::RED,
            true,
        )
        .await?;
        return Ok(());
    };
    let channel_id = ctx.channel_id();
    if ctx.data().games.contains_key(&guild_id) {
        reply_embed(
            ctx,
            "이미 진행 중인 게임이 있습니다.",
            "마피아 게임",
            serenity::Colour::RED,
            true,
        )
        .await?;
        return Ok(());
    }
    if ctx.data().recruitments.contains_key(&guild_id) {
        reply_embed(
            ctx,
            "이미 참가자를 모집 중입니다.",
            "마피아 게임",
            serenity::Colour::RED,
            true,
        )
        .await?;
        return Ok(());
    }
    let config_snapshot = ctx.data().config.read().await.clone();
    if !config_snapshot.game_enabled {
        reply_embed(
            ctx,
            "마피아 게임이 비활성화되어 있습니다.",
            "마피아 게임",
            serenity::Colour::RED,
            true,
        )
        .await?;
        return Ok(());
    }
    let Some(participant_role) = role_by_name(
        ctx.serenity_context(),
        guild_id,
        &config_snapshot.participant_role,
    )
    .await?
    else {
        reply_embed(
            ctx,
            format!(
                "'{}' 역할을 찾을 수 없습니다.",
                config_snapshot.participant_role
            ),
            "마피아 게임",
            serenity::Colour::RED,
            true,
        )
        .await?;
        return Ok(());
    };

    let special_roles = choose_special_roles(&config_snapshot)?;
    let mut role_counts = selected_role_counts(&config_snapshot, &special_roles)?;
    let minimum_players = minimum_player_count(&role_counts);
    let max_players = effective_max_player_count(&config_snapshot);
    if max_players < minimum_players {
        reply_embed(
            ctx,
            format!("현재 설정의 최소 시작 인원은 {minimum_players}명이라 최대 인원 {max_players}명으로 시작할 수 없습니다."),
            "마피아 게임",
            serenity::Colour::RED,
            true,
        )
        .await?;
        return Ok(());
    }
    let done = Arc::new(Notify::new());
    let recruitment = Arc::new(RwLock::new(Recruitment {
        host_user_id: ctx.author().id,
        participant_role_id: participant_role.id,
        role_counts: role_counts.clone(),
        special_roles: special_roles.clone(),
        max_players,
        minimum_players,
        joined_ids: HashSet::new(),
        joined_names: HashMap::new(),
        spectator_ids: HashSet::new(),
        spectator_names: HashMap::new(),
        accepting: true,
        cancelled: false,
        done: done.clone(),
    }));
    ctx.data()
        .recruitments
        .insert(guild_id, recruitment.clone());

    let mention = role_by_name(ctx.serenity_context(), guild_id, GAME_NOTIFICATION_ROLE)
        .await?
        .map(|role| role.mention().to_string());
    let rec = recruitment.read().await;
    let mut reply = poise::CreateReply::default()
        .embed(recruitment_embed(&rec, &config_snapshot, "모집 중입니다."))
        .components(recruitment_components(guild_id, false));
    if let Some(mention) = mention {
        reply = reply.content(mention);
    }
    drop(rec);
    ctx.send(reply).await?;

    tokio::select! {
        _ = tokio::time::sleep(Duration::from_secs(RECRUITMENT_SECONDS)) => {}
        _ = done.notified() => {}
    }

    let mut rec = recruitment.write().await;
    rec.accepting = false;
    let cancelled = rec.cancelled || rec.joined_ids.len() < rec.minimum_players;
    rec.cancelled = cancelled;
    let player_data = rec
        .joined_ids
        .iter()
        .map(|id| {
            (
                *id,
                rec.joined_names
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| id.to_string()),
            )
        })
        .collect::<Vec<_>>();
    if cancelled {
        ctx.data().recruitments.remove(&guild_id);
        reply_embed(
            ctx,
            "참가자 모집이 취소되었습니다.",
            "참가자 모집 취소",
            serenity::Colour::RED,
            false,
        )
        .await?;
        return Ok(());
    }
    let mut game_special_roles = expand_special_roles(&rec.special_roles);
    if config_snapshot.enable_cult_team {
        game_special_roles.extend([Role::CultLeader, Role::Fanatic]);
        *role_counts.entry(Role::CultLeader).or_default() += 1;
        *role_counts.entry(Role::Fanatic).or_default() += 1;
    }
    let participant_user_ids = rec.joined_ids.clone();
    let spectator_user_ids = rec.spectator_ids.clone();
    drop(rec);
    ctx.data().recruitments.remove(&guild_id);

    let game = MafiaGame::new_with_counts(
        player_data,
        GameCounts {
            mafia_count: *role_counts.get(&Role::Mafia).unwrap_or(&0),
            doctor_count: *role_counts.get(&Role::Doctor).unwrap_or(&0),
            police_count: *role_counts.get(&Role::Police).unwrap_or(&0),
            agent_count: *role_counts.get(&Role::Agent).unwrap_or(&0),
            vigilante_count: *role_counts.get(&Role::Vigilante).unwrap_or(&0),
            joker_count: 0,
            special_roles: game_special_roles,
        },
    )?;
    let initial_roles = game.players.iter().map(|p| (p.user_id, p.role)).collect();
    let running = Arc::new(RwLock::new(RunningGame {
        guild_id,
        channel_id,
        participant_user_ids,
        spectator_user_ids,
        reveal_death_roles: config_snapshot.reveal_death_roles,
        anonymous_enabled: config_snapshot.anonymous_mode,
        game,
        started_at: Instant::now(),
        initial_roles,
        memos: HashMap::new(),
        game_status_message_id: None,
        game_status_text: None,
        anonymous_aliases: HashMap::new(),
        anonymous_original_names: HashMap::new(),
        anonymous_input_channel_ids: HashMap::new(),
        anonymous_input_channel_owners: HashMap::new(),
        anonymous_dead_input_channel_ids: HashMap::new(),
        anonymous_dead_input_channel_owners: HashMap::new(),
        anonymous_shaman_input_channel_ids: HashMap::new(),
        anonymous_shaman_input_channel_owners: HashMap::new(),
        anonymous_role_input_channel_ids: HashMap::new(),
        anonymous_role_input_channels: HashMap::new(),
        anonymous_role_input_status_message_ids: HashMap::new(),
        anonymous_role_status_texts: HashMap::new(),
        anonymous_channel_topics: HashMap::new(),
        anonymous_webhook_urls: HashMap::new(),
        original_game_channel_overwrites: HashMap::new(),
        game_channel_overwrites: HashMap::new(),
        member_channel_overwrites: HashMap::new(),
        original_slowmode_delays: HashMap::new(),
        private_channel_ids: HashMap::new(),
        private_role_status_message_ids: HashMap::new(),
        private_role_status_texts: HashMap::new(),
        memo_channel_ids: HashMap::new(),
        shaman_channel_id: None,
        shaman_status_message_id: None,
        shaman_status_text: None,
        frog_channel_id: None,
        frog_game_channel_overwrites: HashMap::new(),
        madam_seduction_channel_overwrites: HashMap::new(),
        day_chat_open: false,
        final_defense_user_id: None,
        day_skip_voter_ids: HashSet::new(),
        day_skip_confirmed: false,
        day_extension_voter_ids: HashSet::new(),
        day_extension_active: false,
        day_extension_confirmed: false,
        night_timed_events_due: false,
        contractor_contract_drafts: HashMap::new(),
        night_notify: Arc::new(Notify::new()),
        vote_notify: Arc::new(Notify::new()),
        confirm_notify: Arc::new(Notify::new()),
        day_notify: Arc::new(Notify::new()),
        stats_recorded: false,
    }));
    ctx.data().games.insert(guild_id, running.clone());
    let data = ctx.data().clone();
    let serenity_ctx = ctx.serenity_context().clone();
    tokio::spawn(async move {
        if let Err(error) = game_loop(serenity_ctx, data, running).await {
            eprintln!("Rust game loop error: {error:?}");
        }
    });

    let running = ctx.data().games.get(&guild_id).unwrap();
    let game = &running.read().await.game;
    reply_embed(
        ctx,
        format!(
            "게임을 시작합니다. 참가자 {}명에게 역할을 DM으로 보냅니다.\n{}",
            game.players.len(),
            public_role_count_text(game)
        ),
        "게임 시작",
        serenity::Colour::DARK_GREEN,
        false,
    )
    .await?;
    Ok(())
}

pub async fn handle_component(
    ctx: &serenity::Context,
    data: &Data,
    component: &serenity::ComponentInteraction,
) -> Result<()> {
    let custom_id = component.data.custom_id.as_str();
    let parts = custom_id.split(':').collect::<Vec<_>>();
    match parts.as_slice() {
        ["join", guild] => handle_join(ctx, data, component, parse_guild(guild)?).await?,
        ["spectate", guild] => handle_spectate(ctx, data, component, parse_guild(guild)?).await?,
        ["startnow", guild] => {
            handle_recruitment_finish(ctx, data, component, parse_guild(guild)?, false).await?
        }
        ["cancelrec", guild] => {
            handle_recruitment_finish(ctx, data, component, parse_guild(guild)?, true).await?
        }
        ["night", guild, actor_id, _role] => {
            handle_night_action(ctx, data, component, parse_guild(guild)?, actor_id.parse()?)
                .await?
        }
        ["contractor_target", guild, actor_id, slot] => {
            handle_contractor_target(
                ctx,
                data,
                component,
                parse_guild(guild)?,
                actor_id.parse()?,
                slot.parse()?,
            )
            .await?
        }
        ["contractor_role", guild, actor_id, slot] => {
            handle_contractor_role(
                ctx,
                data,
                component,
                parse_guild(guild)?,
                actor_id.parse()?,
                slot.parse()?,
            )
            .await?
        }
        ["contractor_submit", guild, actor_id] => {
            handle_contractor_submit(ctx, data, component, parse_guild(guild)?, actor_id.parse()?)
                .await?
        }
        ["vote", guild] => handle_day_vote(ctx, data, component, parse_guild(guild)?).await?,
        ["confirm", guild, approve] => {
            handle_confirm_vote(ctx, data, component, parse_guild(guild)?, *approve == "1").await?
        }
        ["skipday", guild] => handle_skip_day(ctx, data, component, parse_guild(guild)?).await?,
        ["extendday", guild] => {
            handle_day_extension(ctx, data, component, parse_guild(guild)?).await?
        }
        ["hacker", guild, actor_id] => {
            handle_hacker(ctx, data, component, parse_guild(guild)?, actor_id.parse()?).await?
        }
        ["vigilante", guild, actor_id] => {
            handle_vigilante(ctx, data, component, parse_guild(guild)?, actor_id.parse()?).await?
        }
        ["psychologist", guild, actor_id] => {
            handle_psychologist(ctx, data, component, parse_guild(guild)?, actor_id.parse()?)
                .await?
        }
        ["thief", guild, actor_id] => {
            handle_thief(ctx, data, component, parse_guild(guild)?, actor_id.parse()?).await?
        }
        _ => ack_component(ctx, component).await,
    }
    Ok(())
}

pub fn parse_guild(value: &str) -> Result<serenity::GuildId> {
    Ok(serenity::GuildId::new(value.parse()?))
}

pub fn selected_values(component: &serenity::ComponentInteraction) -> Vec<String> {
    match &component.data.kind {
        serenity::ComponentInteractionDataKind::StringSelect { values } => values.clone(),
        _ => Vec::new(),
    }
}

pub async fn handle_contractor_target(
    ctx: &serenity::Context,
    data: &Data,
    component: &serenity::ComponentInteraction,
    guild_id: serenity::GuildId,
    actor_id: u64,
    slot: usize,
) -> Result<()> {
    if component.user.id.get() != actor_id {
        send_component_private(ctx, component, "본인에게 온 선택지만 사용할 수 있습니다.").await?;
        return Ok(());
    }
    if slot >= 2 {
        send_component_private(ctx, component, "잘못된 청부 선택입니다.").await?;
        return Ok(());
    }
    let Some(target_id) = selected_values(component)
        .first()
        .and_then(|value| value.parse().ok())
    else {
        send_component_private(ctx, component, "청부 대상을 선택해야 합니다.").await?;
        return Ok(());
    };
    let Some(running) = data.games.get(&guild_id).map(|entry| entry.clone()) else {
        send_component_private(ctx, component, "진행 중인 게임이 없습니다.").await?;
        return Ok(());
    };
    running
        .write()
        .await
        .contractor_contract_drafts
        .entry(actor_id)
        .or_default()
        .target_ids[slot] = Some(target_id);
    ack_component(ctx, component).await;
    Ok(())
}

pub async fn handle_contractor_role(
    ctx: &serenity::Context,
    data: &Data,
    component: &serenity::ComponentInteraction,
    guild_id: serenity::GuildId,
    actor_id: u64,
    slot: usize,
) -> Result<()> {
    if component.user.id.get() != actor_id {
        send_component_private(ctx, component, "본인에게 온 선택지만 사용할 수 있습니다.").await?;
        return Ok(());
    }
    if slot >= 2 {
        send_component_private(ctx, component, "잘못된 청부 선택입니다.").await?;
        return Ok(());
    }
    let Some(role) = selected_values(component)
        .first()
        .and_then(|value| find_role_by_name(value))
    else {
        send_component_private(ctx, component, "청부 대상 직업을 선택해야 합니다.").await?;
        return Ok(());
    };
    if !CONTRACTOR_GUESS_ROLES.contains(&role) {
        send_component_private(ctx, component, "청부로 추측할 수 없는 직업입니다.").await?;
        return Ok(());
    }
    let Some(running) = data.games.get(&guild_id).map(|entry| entry.clone()) else {
        send_component_private(ctx, component, "진행 중인 게임이 없습니다.").await?;
        return Ok(());
    };
    running
        .write()
        .await
        .contractor_contract_drafts
        .entry(actor_id)
        .or_default()
        .guessed_roles[slot] = Some(role);
    ack_component(ctx, component).await;
    Ok(())
}

pub async fn handle_contractor_submit(
    ctx: &serenity::Context,
    data: &Data,
    component: &serenity::ComponentInteraction,
    guild_id: serenity::GuildId,
    actor_id: u64,
) -> Result<()> {
    if component.user.id.get() != actor_id {
        send_component_private(ctx, component, "본인에게 온 선택지만 사용할 수 있습니다.").await?;
        return Ok(());
    }
    let Some(running) = data.games.get(&guild_id).map(|entry| entry.clone()) else {
        send_component_private(ctx, component, "진행 중인 게임이 없습니다.").await?;
        return Ok(());
    };
    let (message, done, newly_contacted_mafia) = {
        let mut running_write = running.write().await;
        let was_known_mafia_team = running_write
            .game
            .get_player(actor_id)
            .is_some_and(|actor| running_write.game.is_known_mafia_team(actor));
        let Some(draft) = running_write
            .contractor_contract_drafts
            .get(&actor_id)
            .cloned()
        else {
            send_component_private(
                ctx,
                component,
                "청부 대상 2명과 각 대상의 직업을 모두 선택하세요.",
            )
            .await?;
            return Ok(());
        };
        let (Some(first_target_id), Some(second_target_id), Some(first_role), Some(second_role)) = (
            draft.target_ids[0],
            draft.target_ids[1],
            draft.guessed_roles[0],
            draft.guessed_roles[1],
        ) else {
            send_component_private(
                ctx,
                component,
                "청부 대상 2명과 각 대상의 직업을 모두 선택하세요.",
            )
            .await?;
            return Ok(());
        };
        let message = match running_write.game.submit_contractor_contract(
            actor_id,
            first_target_id,
            first_role,
            second_target_id,
            second_role,
        ) {
            Ok(message) => message,
            Err(error) => {
                send_component_private(ctx, component, error.to_string()).await?;
                return Ok(());
            }
        };
        running_write.contractor_contract_drafts.remove(&actor_id);
        let newly_contacted_mafia = running_write
            .game
            .get_player(actor_id)
            .filter(|actor| {
                actor.alive
                    && !was_known_mafia_team
                    && running_write.game.is_known_mafia_team(actor)
            })
            .cloned();
        let done = running_write.game.should_finish_night_early();
        (message, done, newly_contacted_mafia)
    };
    if let Some(player) = &newly_contacted_mafia {
        grant_private_role_member_access(ctx, data, &running, Role::Mafia, player).await;
    }
    if done {
        running.read().await.night_notify.notify_waiters();
    }
    component
        .create_response(
            ctx,
            serenity::CreateInteractionResponse::UpdateMessage(
                serenity::CreateInteractionResponseMessage::new()
                    .embed(make_embed(
                        message,
                        "밤 행동 완료",
                        serenity::Colour::DARK_GREEN,
                    ))
                    .components(vec![]),
            ),
        )
        .await?;
    if running.read().await.night_timed_events_due {
        trigger_timed_night_events(ctx, data, &running).await?;
    }
    Ok(())
}

pub async fn handle_skip_day(
    ctx: &serenity::Context,
    data: &Data,
    component: &serenity::ComponentInteraction,
    guild_id: serenity::GuildId,
) -> Result<()> {
    let Some(running) = data.games.get(&guild_id).map(|entry| entry.clone()) else {
        send_component_private(ctx, component, "진행 중인 게임이 없습니다.").await?;
        return Ok(());
    };
    let user_id = component.user.id.get();
    let outcome = {
        let mut running_write = running.write().await;
        if running_write.game.phase != Phase::Day {
            return send_component_private(ctx, component, "지금 진행 중인 낮 토론이 없습니다.")
                .await
                .map_err(Into::into);
        }
        let alive_ids = running_write
            .game
            .alive_players()
            .into_iter()
            .map(|player| player.user_id)
            .collect::<HashSet<_>>();
        if !alive_ids.contains(&user_id) {
            return send_component_private(
                ctx,
                component,
                "생존 중인 참가자만 바로 투표를 선택할 수 있습니다.",
            )
            .await
            .map_err(Into::into);
        }
        let required_votes = alive_ids.len() / 2 + 1;
        if running_write.day_skip_voter_ids.contains(&user_id) {
            return send_component_private(
                ctx,
                component,
                format!(
                    "이미 바로 투표에 동의했습니다. 현재 {}/{}명",
                    running_write.day_skip_voter_ids.len(),
                    required_votes
                ),
            )
            .await
            .map_err(Into::into);
        }
        running_write.day_skip_voter_ids.insert(user_id);
        let vote_count = running_write.day_skip_voter_ids.len();
        if vote_count < required_votes {
            return send_component_private(
                ctx,
                component,
                format!("바로 투표에 동의했습니다. 현재 {vote_count}/{required_votes}명"),
            )
            .await
            .map_err(Into::into);
        }
        running_write.day_skip_confirmed = true;
        running_write.day_extension_active = false;
        (
            vote_count,
            alive_ids.len(),
            running_write.day_notify.clone(),
            running_write.guild_id,
        )
    };
    let (vote_count, alive_count, notify, guild_id) = outcome;
    notify.notify_waiters();
    component
        .create_response(
            ctx,
            serenity::CreateInteractionResponse::UpdateMessage(
                serenity::CreateInteractionResponseMessage::new()
                    .embed(make_embed(
                        format!(
                            "생존자 과반수가 바로 투표를 선택했습니다. ({vote_count}/{alive_count}명)\n토론을 끝내고 바로 지목 투표로 넘어갑니다."
                        ),
                        "바로 투표",
                        serenity::Colour::DARK_GREEN,
                    ))
                    .components(day_skip_components(guild_id, true, true)),
            ),
        )
        .await?;
    if running.read().await.night_timed_events_due {
        trigger_timed_night_events(ctx, data, &running).await?;
    }
    Ok(())
}

pub async fn handle_day_extension(
    ctx: &serenity::Context,
    data: &Data,
    component: &serenity::ComponentInteraction,
    guild_id: serenity::GuildId,
) -> Result<()> {
    let Some(running) = data.games.get(&guild_id).map(|entry| entry.clone()) else {
        send_component_private(ctx, component, "진행 중인 게임이 없습니다.").await?;
        return Ok(());
    };
    let user_id = component.user.id.get();
    let outcome = {
        let mut running_write = running.write().await;
        if !running_write.day_extension_active {
            return send_component_private(ctx, component, "연장 투표가 종료되었습니다.")
                .await
                .map_err(Into::into);
        }
        if running_write.game.phase != Phase::Day {
            return send_component_private(ctx, component, "지금 진행 중인 낮 토론이 없습니다.")
                .await
                .map_err(Into::into);
        }
        let alive_ids = running_write
            .game
            .alive_players()
            .into_iter()
            .map(|player| player.user_id)
            .collect::<HashSet<_>>();
        if !alive_ids.contains(&user_id) {
            return send_component_private(
                ctx,
                component,
                "생존 중인 참가자만 연장 투표를 할 수 있습니다.",
            )
            .await
            .map_err(Into::into);
        }
        let required_votes = alive_ids.len() / 2 + 1;
        if running_write.day_extension_voter_ids.contains(&user_id) {
            return send_component_private(
                ctx,
                component,
                format!(
                    "이미 1분 연장에 투표했습니다. 현재 {}/{}명",
                    running_write.day_extension_voter_ids.len(),
                    required_votes
                ),
            )
            .await
            .map_err(Into::into);
        }
        running_write.day_extension_voter_ids.insert(user_id);
        let vote_count = running_write.day_extension_voter_ids.len();
        if vote_count < required_votes {
            return send_component_private(
                ctx,
                component,
                format!("1분 연장에 투표했습니다. 현재 {vote_count}/{required_votes}명"),
            )
            .await
            .map_err(Into::into);
        }
        running_write.day_extension_confirmed = true;
        running_write.day_extension_active = false;
        (
            vote_count,
            alive_ids.len(),
            running_write.day_notify.clone(),
            running_write.guild_id,
        )
    };
    let (vote_count, alive_count, notify, guild_id) = outcome;
    notify.notify_waiters();
    component
        .create_response(
            ctx,
            serenity::CreateInteractionResponse::UpdateMessage(
                serenity::CreateInteractionResponseMessage::new()
                    .embed(make_embed(
                        format!(
                            "생존자 과반수가 1분 연장을 선택했습니다. ({vote_count}/{alive_count}명)\n낮 토론을 1분 연장합니다."
                        ),
                        "낮 토론 연장",
                        serenity::Colour::DARK_GREEN,
                    ))
                    .components(day_extension_components(guild_id, true, true)),
            ),
        )
        .await?;
    if running.read().await.night_timed_events_due {
        trigger_timed_night_events(ctx, data, &running).await?;
    }
    Ok(())
}

pub async fn handle_join(
    ctx: &serenity::Context,
    data: &Data,
    component: &serenity::ComponentInteraction,
    guild_id: serenity::GuildId,
) -> Result<()> {
    let Some(recruitment) = data.recruitments.get(&guild_id).map(|entry| entry.clone()) else {
        send_component_private(ctx, component, "참가자 모집이 종료되었습니다.").await?;
        return Ok(());
    };
    let mut rec = recruitment.write().await;
    if !rec.accepting {
        send_component_private(ctx, component, "참가자 모집이 종료되었습니다.").await?;
        return Ok(());
    }
    let user_id = component.user.id.get();
    let config_snapshot = data.config.read().await;
    if is_blacklisted(&config_snapshot, user_id) {
        send_component_private(
            ctx,
            component,
            "블랙리스트에 등록된 유저는 참가할 수 없습니다.",
        )
        .await?;
        return Ok(());
    }
    drop(config_snapshot);
    if rec.joined_ids.contains(&user_id) {
        send_component_private(ctx, component, "이미 참가했습니다.").await?;
        return Ok(());
    }
    if rec.spectator_ids.contains(&user_id) {
        send_component_private(ctx, component, "이미 관전자로 등록되어 있습니다.").await?;
        return Ok(());
    }
    if rec.joined_ids.len() >= rec.max_players {
        send_component_private(
            ctx,
            component,
            format!(
                "최대 참가 인원 {}명에 도달해 더 이상 참가할 수 없습니다.",
                rec.max_players
            ),
        )
        .await?;
        return Ok(());
    }
    if let Some(member) = component.member.clone() {
        let _ = member.add_role(ctx, rec.participant_role_id).await;
        rec.joined_names.insert(user_id, display_name(&member));
    } else {
        rec.joined_names
            .insert(user_id, component.user.name.clone());
    }
    rec.joined_ids.insert(user_id);
    let updated = rec.clone();
    drop(rec);
    send_component_private(ctx, component, "참가 완료!").await?;
    update_recruitment_message(
        ctx,
        data,
        component,
        guild_id,
        &updated,
        RECRUITMENT_STATUS_OPEN,
        false,
    )
    .await;
    Ok(())
}

pub async fn handle_spectate(
    ctx: &serenity::Context,
    data: &Data,
    component: &serenity::ComponentInteraction,
    guild_id: serenity::GuildId,
) -> Result<()> {
    let Some(recruitment) = data.recruitments.get(&guild_id).map(|entry| entry.clone()) else {
        send_component_private(ctx, component, "참가자 모집이 종료되었습니다.").await?;
        return Ok(());
    };
    let mut rec = recruitment.write().await;
    if !rec.accepting {
        send_component_private(ctx, component, "참가자 모집이 종료되었습니다.").await?;
        return Ok(());
    }
    let user_id = component.user.id.get();
    if rec.joined_ids.contains(&user_id) {
        send_component_private(ctx, component, "이미 참가자로 등록되어 있습니다.").await?;
        return Ok(());
    }
    if rec.spectator_ids.contains(&user_id) {
        send_component_private(ctx, component, "이미 관전자로 등록되어 있습니다.").await?;
        return Ok(());
    }
    rec.spectator_ids.insert(user_id);
    if let Some(member) = component.member.clone() {
        rec.spectator_names.insert(user_id, display_name(&member));
        if let Some(role) = role_by_name(ctx, guild_id, SPECTATOR_ROLE).await? {
            let _ = member.add_role(ctx, role.id).await;
        }
    } else {
        rec.spectator_names
            .insert(user_id, component.user.name.clone());
    }
    let updated = rec.clone();
    drop(rec);
    send_component_private(ctx, component, "관전 등록 완료!").await?;
    update_recruitment_message(
        ctx,
        data,
        component,
        guild_id,
        &updated,
        RECRUITMENT_STATUS_OPEN,
        false,
    )
    .await;
    Ok(())
}

pub async fn handle_recruitment_finish(
    ctx: &serenity::Context,
    data: &Data,
    component: &serenity::ComponentInteraction,
    guild_id: serenity::GuildId,
    cancelled: bool,
) -> Result<()> {
    let Some(recruitment) = data.recruitments.get(&guild_id).map(|entry| entry.clone()) else {
        send_component_private(ctx, component, "참가자 모집이 이미 종료되었습니다.").await?;
        return Ok(());
    };
    let mut rec = recruitment.write().await;
    if component.user.id != rec.host_user_id {
        send_component_private(ctx, component, "게임을 모집한 주최자만 사용할 수 있습니다.")
            .await?;
        return Ok(());
    }
    if !cancelled && rec.joined_ids.len() < rec.minimum_players {
        send_component_private(
            ctx,
            component,
            format!(
                "아직 시작할 수 없습니다. 최소 {}명이 필요합니다. 현재 {}명입니다.",
                rec.minimum_players,
                rec.joined_ids.len()
            ),
        )
        .await?;
        return Ok(());
    }
    rec.cancelled = cancelled;
    rec.accepting = false;
    let updated = rec.clone();
    rec.done.notify_waiters();
    drop(rec);
    if cancelled {
        ack_component(ctx, component).await;
        update_recruitment_message(
            ctx,
            data,
            component,
            guild_id,
            &updated,
            RECRUITMENT_STATUS_CANCELLED,
            true,
        )
        .await;
    } else {
        ack_component(ctx, component).await;
    }
    Ok(())
}

pub async fn handle_night_action(
    ctx: &serenity::Context,
    data: &Data,
    component: &serenity::ComponentInteraction,
    guild_id: serenity::GuildId,
    actor_id: u64,
) -> Result<()> {
    if component.user.id.get() != actor_id {
        send_component_private(ctx, component, "본인에게 온 선택지만 사용할 수 있습니다.").await?;
        return Ok(());
    }
    let Some(running) = data.games.get(&guild_id).map(|entry| entry.clone()) else {
        send_component_private(ctx, component, "진행 중인 게임이 없습니다.").await?;
        return Ok(());
    };
    let values = selected_values(component);
    let target_id = values.first().and_then(|value| {
        if value == "skip" {
            None
        } else {
            value.parse().ok()
        }
    });
    let (
        message,
        done,
        mafia_action_view,
        spy_bonus_targets,
        newly_contacted_mafia,
        cult_bells,
        immediate_police_result,
        broadcast_police_result,
    ) = {
        let mut running_write = running.write().await;
        let was_known_mafia_team = running_write
            .game
            .get_player(actor_id)
            .is_some_and(|actor| running_write.game.is_known_mafia_team(actor));
        let message = match running_write.game.submit_night_action(actor_id, target_id) {
            Ok(message) => message,
            Err(error) => {
                send_component_private(ctx, component, error.to_string()).await?;
                return Ok(());
            }
        };
        let cult_bells = running_write.game.consume_cult_bells();
        let actor = running_write.game.get_player(actor_id).cloned();
        let is_police_action = actor.as_ref().is_some_and(|actor| {
            actor.role == Role::Police
                || (actor.role == Role::Thief
                    && running_write.game.thief_night_role(actor) == Some(Role::Police))
        });
        let (immediate_police_result, broadcast_police_result) = if is_police_action {
            if let Some(result) = running_write.game.consume_ready_police_result() {
                (Some(result.clone()), Some(result))
            } else {
                (
                    Some(
                        "다른 경찰의 선택이 남아 있어 조사 결과는 아직 확정되지 않았습니다."
                            .to_string(),
                    ),
                    None,
                )
            }
        } else {
            (None, None)
        };
        let newly_contacted_mafia = actor
            .as_ref()
            .filter(|actor| {
                actor.alive
                    && !was_known_mafia_team
                    && running_write.game.is_known_mafia_team(actor)
            })
            .cloned();
        let mafia_action_view = actor.as_ref().and_then(|actor| {
            let role = effective_night_role(&running_write.game, actor);
            if actor.role == Role::Mafia || (actor.role == Role::Thief && role == Role::Mafia) {
                Some((
                    night_targets(&running_write.game, actor),
                    mafia_night_target_status_text(&running_write),
                ))
            } else {
                None
            }
        });
        let spy_bonus_targets = actor.and_then(|actor| {
            if actor.role == Role::Spy && running_write.game.spy_can_use_bonus_action(actor_id) {
                Some(night_targets(&running_write.game, &actor))
            } else {
                None
            }
        });
        let done = running_write.game.should_finish_night_early();
        (
            message,
            done,
            mafia_action_view,
            spy_bonus_targets,
            newly_contacted_mafia,
            cult_bells,
            immediate_police_result,
            broadcast_police_result,
        )
    };
    if let Some(player) = &newly_contacted_mafia {
        grant_private_role_member_access(ctx, data, &running, Role::Mafia, player).await;
    }
    if let Some(result) = &broadcast_police_result {
        send_police_result_message(ctx, &running, result, Some(actor_id)).await;
    }
    let response_message = if let Some(result) = immediate_police_result {
        format!("{message}\n\n{result}")
    } else {
        message
    };
    if let Some((targets, status_text)) = mafia_action_view {
        component
            .create_response(
                ctx,
                serenity::CreateInteractionResponse::UpdateMessage(
                    serenity::CreateInteractionResponseMessage::new()
                        .embed(make_embed(
                            format!("{response_message}\n\n{status_text}"),
                            "마피아 처치 선택",
                            serenity::Colour::DARK_GREEN,
                        ))
                        .components(night_action_components(
                            guild_id,
                            actor_id,
                            Role::Mafia,
                            &targets,
                        )),
                ),
            )
            .await?;
        upsert_private_role_status_message(ctx, &running, Role::Mafia).await;
        if running.read().await.night_timed_events_due {
            trigger_timed_night_events(ctx, data, &running).await?;
        }
        return Ok(());
    }
    if let Some(targets) = spy_bonus_targets {
        component
            .create_response(
                ctx,
                serenity::CreateInteractionResponse::UpdateMessage(
                    serenity::CreateInteractionResponseMessage::new()
                        .embed(make_embed(
                            format!(
                                "{response_message}\n\n추가 첩보를 한 번 더 사용할 수 있습니다."
                            ),
                            "접선 성공",
                            serenity::Colour::DARK_GREEN,
                        ))
                        .components(night_action_components(
                            guild_id,
                            actor_id,
                            Role::Spy,
                            &targets,
                        )),
                ),
            )
            .await?;
        if running.read().await.night_timed_events_due {
            trigger_timed_night_events(ctx, data, &running).await?;
        }
        return Ok(());
    }
    if done {
        running.read().await.night_notify.notify_waiters();
    }
    component
        .create_response(
            ctx,
            serenity::CreateInteractionResponse::UpdateMessage(
                serenity::CreateInteractionResponseMessage::new()
                    .embed(make_embed(
                        response_message,
                        "밤 행동 완료",
                        serenity::Colour::DARK_GREEN,
                    ))
                    .components(vec![]),
            ),
        )
        .await?;
    if running.read().await.night_timed_events_due {
        trigger_timed_night_events(ctx, data, &running).await?;
    }
    if cult_bells > 0 {
        send_game_embed(
            ctx,
            &running,
            std::iter::repeat_n("교주의 종소리가 울렸습니다.", cult_bells as usize)
                .collect::<Vec<_>>()
                .join("\n"),
            "교주 포교",
            serenity::Colour::ORANGE,
            vec![],
            true,
            true,
        )
        .await?;
        sync_cult_team_channel_access(ctx, data, &running).await;
    }
    Ok(())
}

pub async fn handle_day_vote(
    ctx: &serenity::Context,
    data: &Data,
    component: &serenity::ComponentInteraction,
    guild_id: serenity::GuildId,
) -> Result<()> {
    let Some(running) = data.games.get(&guild_id).map(|entry| entry.clone()) else {
        send_component_private(ctx, component, "진행 중인 게임이 없습니다.").await?;
        return Ok(());
    };
    let values = selected_values(component);
    let target_id = values.first().and_then(|value| {
        if value == "skip" {
            None
        } else {
            value.parse().ok()
        }
    });
    let (message, done) = {
        let mut running_write = running.write().await;
        let message = match running_write
            .game
            .submit_day_vote(component.user.id.get(), target_id)
        {
            Ok(message) => message,
            Err(error) => {
                send_component_private(ctx, component, error.to_string()).await?;
                return Ok(());
            }
        };
        (message, running_write.game.all_day_votes_submitted())
    };
    if done {
        running.read().await.vote_notify.notify_waiters();
    }
    send_component_private(ctx, component, message).await?;
    Ok(())
}

pub async fn handle_confirm_vote(
    ctx: &serenity::Context,
    data: &Data,
    component: &serenity::ComponentInteraction,
    guild_id: serenity::GuildId,
    approve: bool,
) -> Result<()> {
    let Some(running) = data.games.get(&guild_id).map(|entry| entry.clone()) else {
        send_component_private(ctx, component, "진행 중인 게임이 없습니다.").await?;
        return Ok(());
    };
    let (message, done) = {
        let mut running_write = running.write().await;
        let message = match running_write
            .game
            .submit_confirmation_vote(component.user.id.get(), approve)
        {
            Ok(message) => message,
            Err(error) => {
                send_component_private(ctx, component, error.to_string()).await?;
                return Ok(());
            }
        };
        (message, running_write.game.all_confirm_votes_submitted())
    };
    if done {
        running.read().await.confirm_notify.notify_waiters();
    }
    send_component_private(ctx, component, message).await?;
    Ok(())
}

pub async fn handle_hacker(
    ctx: &serenity::Context,
    data: &Data,
    component: &serenity::ComponentInteraction,
    guild_id: serenity::GuildId,
    actor_id: u64,
) -> Result<()> {
    let value = selected_values(component)
        .first()
        .and_then(|v| v.parse().ok());
    handle_day_action(
        ctx,
        data,
        component,
        guild_id,
        actor_id,
        value,
        "해킹 완료",
        |game, actor, target| game.submit_hacker_action(actor, target),
        |_, _, message| format!("{message}\n밤이 시작될 때 대상의 직업을 확인합니다."),
    )
    .await
}

pub async fn handle_vigilante(
    ctx: &serenity::Context,
    data: &Data,
    component: &serenity::ComponentInteraction,
    guild_id: serenity::GuildId,
    actor_id: u64,
) -> Result<()> {
    let value = selected_values(component)
        .first()
        .and_then(|v| v.parse().ok());
    handle_day_action(
        ctx,
        data,
        component,
        guild_id,
        actor_id,
        value,
        "숙청 조사 완료",
        |game, actor, target| game.submit_vigilante_investigation(actor, target),
        |game, actor, message| {
            let investigation = game
                .consume_vigilante_results()
                .remove(&actor)
                .unwrap_or_else(|| "조사 결과를 확인하지 못했습니다.".to_string());
            format!("{message}\n\n{investigation}")
        },
    )
    .await
}

pub async fn handle_thief(
    ctx: &serenity::Context,
    data: &Data,
    component: &serenity::ComponentInteraction,
    guild_id: serenity::GuildId,
    actor_id: u64,
) -> Result<()> {
    let value = selected_values(component)
        .first()
        .and_then(|v| v.parse().ok());
    handle_day_action(
        ctx,
        data,
        component,
        guild_id,
        actor_id,
        value,
        "도벽 완료",
        |game, actor, target| game.submit_thief_steal(actor, target),
        |_, _, message| message,
    )
    .await
}

pub async fn handle_psychologist(
    ctx: &serenity::Context,
    data: &Data,
    component: &serenity::ComponentInteraction,
    guild_id: serenity::GuildId,
    actor_id: u64,
) -> Result<()> {
    if component.user.id.get() != actor_id {
        send_component_private(ctx, component, "본인에게 온 선택지만 사용할 수 있습니다.").await?;
        return Ok(());
    }
    let values = selected_values(component);
    if values.len() < 2 {
        send_component_private(ctx, component, "서로 다른 두 명을 선택해야 합니다.").await?;
        return Ok(());
    }
    let Some(running) = data.games.get(&guild_id).map(|entry| entry.clone()) else {
        send_component_private(ctx, component, "진행 중인 게임이 없습니다.").await?;
        return Ok(());
    };
    let (Some(first), Some(second)) = (
        values.first().and_then(|value| value.parse().ok()),
        values.get(1).and_then(|value| value.parse().ok()),
    ) else {
        ack_component(ctx, component).await;
        return Ok(());
    };
    let message = {
        let mut running_write = running.write().await;
        match running_write
            .game
            .submit_psychologist_observation(actor_id, first, second)
        {
            Ok(message) => message,
            Err(error) => {
                send_component_private(ctx, component, error.to_string()).await?;
                return Ok(());
            }
        }
    };
    ack_component(ctx, component).await;
    component
        .channel_id
        .edit_message(
            &ctx.http,
            component.message.id,
            serenity::EditMessage::new()
                .embed(make_embed(
                    message,
                    "심리학자 관찰 완료",
                    serenity::Colour::DARK_GREEN,
                ))
                .components(vec![]),
        )
        .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn handle_day_action<F, G>(
    ctx: &serenity::Context,
    data: &Data,
    component: &serenity::ComponentInteraction,
    guild_id: serenity::GuildId,
    actor_id: u64,
    target_id: Option<u64>,
    title: &'static str,
    apply: F,
    finish_message: G,
) -> Result<()>
where
    F: FnOnce(&mut MafiaGame, u64, u64) -> Result<String>,
    G: FnOnce(&mut MafiaGame, u64, String) -> String,
{
    if component.user.id.get() != actor_id {
        send_component_private(ctx, component, "본인에게 온 선택지만 사용할 수 있습니다.").await?;
        return Ok(());
    }
    let Some(target_id) = target_id else {
        send_component_private(ctx, component, "대상을 선택해야 합니다.").await?;
        return Ok(());
    };
    let Some(running) = data.games.get(&guild_id).map(|entry| entry.clone()) else {
        send_component_private(ctx, component, "진행 중인 게임이 없습니다.").await?;
        return Ok(());
    };
    let (message, newly_contacted_mafia) = {
        let mut running_write = running.write().await;
        let was_known_mafia_team = running_write
            .game
            .get_player(actor_id)
            .is_some_and(|actor| running_write.game.is_known_mafia_team(actor));
        let message = match apply(&mut running_write.game, actor_id, target_id) {
            Ok(message) => message,
            Err(error) => {
                send_component_private(ctx, component, error.to_string()).await?;
                return Ok(());
            }
        };
        let message = finish_message(&mut running_write.game, actor_id, message);
        let newly_contacted_mafia = running_write
            .game
            .get_player(actor_id)
            .filter(|actor| {
                actor.alive
                    && !was_known_mafia_team
                    && running_write.game.is_known_mafia_team(actor)
            })
            .cloned();
        (message, newly_contacted_mafia)
    };
    if let Some(player) = &newly_contacted_mafia {
        grant_private_role_member_access(ctx, data, &running, Role::Mafia, player).await;
    }
    component
        .create_response(
            ctx,
            serenity::CreateInteractionResponse::UpdateMessage(
                serenity::CreateInteractionResponseMessage::new()
                    .embed(make_embed(message, title, serenity::Colour::DARK_GREEN))
                    .components(vec![]),
            ),
        )
        .await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "마피아중지",
    description_localized("ko", "진행 중인 마피아 게임을 중지합니다.")
)]
pub async fn stop_game(ctx: Context<'_>) -> Result<(), Error> {
    if !require_manager(ctx).await? {
        return Ok(());
    }
    let Some(guild_id) = ctx.guild_id() else {
        return Ok(());
    };
    if let Some((_id, running)) = ctx.data().games.remove(&guild_id) {
        let (roles, notifies) = {
            let mut running_write = running.write().await;
            running_write.game.phase = Phase::Ended;
            (
                running_write.game.reveal_roles(),
                [
                    running_write.night_notify.clone(),
                    running_write.vote_notify.clone(),
                    running_write.confirm_notify.clone(),
                    running_write.day_notify.clone(),
                ],
            )
        };
        for notify in notifies {
            notify.notify_waiters();
        }
        send_game_embed(
            ctx.serenity_context(),
            &running,
            format!("관리자가 게임을 중지했습니다.\n\n최종 역할\n{roles}"),
            "게임 중지",
            serenity::Colour::RED,
            vec![],
            true,
            true,
        )
        .await?;
        cleanup_game(ctx.serenity_context(), ctx.data(), &running).await;
        reply_embed(
            ctx,
            "게임을 중지했습니다.",
            "게임 중지",
            serenity::Colour::DARK_GREEN,
            false,
        )
        .await?;
    } else {
        reply_embed(
            ctx,
            "진행 중인 게임이 없습니다.",
            "마피아 게임",
            serenity::Colour::RED,
            true,
        )
        .await?;
    }
    Ok(())
}

pub async fn show_public_status_impl(ctx: Context<'_>) -> Result<(), Error> {
    let Some(guild_id) = ctx.guild_id() else {
        reply_embed(
            ctx,
            "서버에서만 사용할 수 있습니다.",
            "마피아 게임",
            serenity::Colour::RED,
            true,
        )
        .await?;
        return Ok(());
    };
    let Some(running) = ctx.data().games.get(&guild_id).map(|entry| entry.clone()) else {
        reply_embed(
            ctx,
            "진행 중인 게임이 없습니다.",
            "마피아 게임",
            serenity::Colour::RED,
            true,
        )
        .await?;
        return Ok(());
    };
    let (text, ephemeral) = {
        let running_read = running.read().await;
        (
            command_status_text(&running_read, ctx.author().id.get()),
            running_read.anonymous_enabled
                && running_read
                    .game
                    .get_player(ctx.author().id.get())
                    .is_some(),
        )
    };
    reply_embed(ctx, text, "게임 현황", serenity::Colour::GOLD, ephemeral).await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "상태",
    description_localized("ko", "현재 마피아 게임 생존자와 사망자를 확인합니다.")
)]
pub async fn show_public_status(ctx: Context<'_>) -> Result<(), Error> {
    show_public_status_impl(ctx).await
}

#[poise::command(
    slash_command,
    rename = "마피아상태",
    description_localized("ko", "진행 중인 마피아 게임 상태를 확인합니다.")
)]
pub async fn show_manager_status(ctx: Context<'_>) -> Result<(), Error> {
    if !require_manager(ctx).await? {
        return Ok(());
    }
    let Some(guild_id) = ctx.guild_id() else {
        reply_embed(
            ctx,
            "서버에서만 사용할 수 있습니다.",
            "마피아 게임",
            serenity::Colour::RED,
            true,
        )
        .await?;
        return Ok(());
    };
    let Some(running) = ctx.data().games.get(&guild_id).map(|entry| entry.clone()) else {
        reply_embed(
            ctx,
            "진행 중인 게임이 없습니다.",
            "마피아 게임",
            serenity::Colour::RED,
            true,
        )
        .await?;
        return Ok(());
    };
    let text = running.read().await.game.public_status();
    reply_embed(ctx, text, "게임 상태", serenity::Colour::GOLD, true).await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "메모",
    description_localized("ko", "개인 메모 채널에 참가자별 메모를 저장하거나 조회합니다.")
)]
pub async fn memo(
    ctx: Context<'_>,
    #[description = "메모 대상 참가자"] 참가자: serenity::User,
    #[description = "저장할 메모 내용. 비워두면 조회합니다."] 메모내용: Option<String>,
) -> Result<(), Error> {
    let Some(guild_id) = ctx.guild_id() else {
        reply_embed(
            ctx,
            "서버에서만 사용할 수 있습니다.",
            "메모",
            serenity::Colour::RED,
            true,
        )
        .await?;
        return Ok(());
    };
    let Some(running) = ctx.data().games.get(&guild_id).map(|entry| entry.clone()) else {
        reply_embed(
            ctx,
            "진행 중인 게임이 없습니다.",
            "메모",
            serenity::Colour::RED,
            true,
        )
        .await?;
        return Ok(());
    };
    let author_id = ctx.author().id.get();
    let (author, target, channel_id) = {
        let running_read = running.read().await;
        let Some(author) = running_read.game.get_player(author_id).cloned() else {
            reply_embed(
                ctx,
                "현재 게임 참가자만 메모를 사용할 수 있습니다.",
                "메모",
                serenity::Colour::RED,
                true,
            )
            .await?;
            return Ok(());
        };
        let Some(target) = running_read.game.get_player(참가자.id.get()).cloned() else {
            reply_embed(
                ctx,
                "메모 대상은 현재 게임 참가자여야 합니다.",
                "메모",
                serenity::Colour::RED,
                true,
            )
            .await?;
            return Ok(());
        };
        (author, target, running_read.channel_id)
    };

    let config = ctx.data().config.read().await.clone();
    let roles = channel_role_ids(
        ctx.serenity_context(),
        guild_id,
        &config,
        ctx.data().bot_user_id,
    )
    .await?;
    let category = source_category(ctx.serenity_context(), channel_id).await;
    let Some(memo_channel_id) =
        ensure_memo_channel(ctx.serenity_context(), &running, &author, roles, category).await
    else {
        reply_embed(
            ctx,
            "개인 메모 채널을 만들 수 없습니다.",
            "메모",
            serenity::Colour::RED,
            true,
        )
        .await?;
        return Ok(());
    };

    let content = 메모내용.unwrap_or_default().trim().to_string();
    if !content.is_empty() {
        let (memo_number, target_name) = {
            let mut running_write = running.write().await;
            let target_name = running_write
                .game
                .get_player(target.user_id)
                .map(|target| status_display_name(&running_write, target))
                .unwrap_or_else(|| target.name.clone());
            let memos = running_write
                .memos
                .entry(author_id)
                .or_default()
                .entry(target.user_id)
                .or_default();
            memos.push(content.clone());
            (memos.len(), target_name)
        };
        let _ = send_channel_embed(
            ctx.http(),
            memo_channel_id,
            format!("대상: {target_name}\n{memo_number}. {content}"),
            "메모 등록",
            serenity::Colour::DARK_GREEN,
            vec![],
        )
        .await;
        reply_embed(
            ctx,
            format!("{target_name} 님에 대한 메모를 저장했습니다."),
            "메모 등록",
            serenity::Colour::DARK_GREEN,
            true,
        )
        .await?;
    } else {
        let chunks = {
            let running_read = running.read().await;
            let target_name = running_read
                .game
                .get_player(target.user_id)
                .map(|target| status_display_name(&running_read, target))
                .unwrap_or_else(|| target.name.clone());
            let memos = running_read
                .memos
                .get(&author_id)
                .and_then(|target_memos| target_memos.get(&target.user_id))
                .cloned()
                .unwrap_or_default();
            let header = format!("{target_name} 님에 대한 메모");
            if memos.is_empty() {
                vec![format!("{header}\n저장된 메모가 없습니다.")]
            } else {
                let mut chunks = Vec::new();
                let mut current = header.clone();
                for (index, memo) in memos.iter().enumerate() {
                    let line = format!("{}. {memo}", index + 1);
                    if current.len() + line.len() + 1 > 3500 {
                        chunks.push(current);
                        current = format!("{header} (계속)\n{line}");
                    } else {
                        current.push('\n');
                        current.push_str(&line);
                    }
                }
                chunks.push(current);
                chunks
            }
        };
        for chunk in chunks {
            ctx.send(
                poise::CreateReply::default()
                    .embed(make_embed(chunk, "메모 조회", serenity::Colour::GOLD))
                    .ephemeral(true),
            )
            .await?;
        }
    }
    Ok(())
}

pub fn personal_stats_text(stats_file: &stats::StatsFile, user_id: u64, fallback_name: &str) -> String {
    let Some(entry) = stats_file.users.get(&user_id.to_string()) else {
        return "아직 기록된 게임 전적이 없습니다.".to_string();
    };
    let name = if entry.name.is_empty() {
        fallback_name
    } else {
        &entry.name
    };
    format!(
        "{name}님의 전적\n전체 게임: **{}판**\n승리/패배: **{}승 {}패**\n승률: **{}**\n마피아팀 플레이: **{}회**\n게임시간: **{}**\n레이팅: **{}점** (최고 {}점, 반영 {}판)\n\n역할별 플레이\n{}",
        entry.games,
        entry.wins,
        entry.losses,
        stats::win_rate_text(entry.wins, entry.games),
        entry.mafia_team_games,
        stats::play_duration_text(entry.play_seconds),
        entry.rating,
        entry.rating_peak,
        entry.rating_games,
        stats::role_stats_text(entry)
    )
}

#[poise::command(
    slash_command,
    rename = "내정보",
    description_localized("ko", "내 마피아 게임 전적을 확인합니다.")
)]
pub async fn show_my_info(ctx: Context<'_>) -> Result<(), Error> {
    let stats_file = ctx.data().stats.read().await;
    let user = ctx.author();
    let text = personal_stats_text(&stats_file, user.id.get(), &user.name);
    reply_embed(ctx, text, "내정보", serenity::Colour::GOLD, true).await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "레이팅로그",
    description_localized("ko", "내 최근 레이팅 변화 기록을 확인합니다.")
)]
pub async fn rating_log(ctx: Context<'_>) -> Result<(), Error> {
    let stats_file = ctx.data().stats.read().await;
    let user = ctx.author();
    let text = stats::rating_log_text(&stats_file, user.id.get(), &user.name, 10);
    reply_embed(ctx, text, "레이팅 로그", serenity::Colour::GOLD, true).await?;
    Ok(())
}

pub fn image_color(hex: &str) -> Rgb<u8> {
    let value = hex.trim_start_matches('#');
    let red = u8::from_str_radix(&value[0..2], 16).unwrap_or(255);
    let green = u8::from_str_radix(&value[2..4], 16).unwrap_or(255);
    let blue = u8::from_str_radix(&value[4..6], 16).unwrap_or(255);
    Rgb([red, green, blue])
}

pub fn fill_rect(image: &mut RgbImage, x: i32, y: i32, width: u32, height: u32, color: Rgb<u8>) {
    let left = x.max(0) as u32;
    let top = y.max(0) as u32;
    let right = (x as i64 + width as i64)
        .clamp(0, image.width() as i64)
        .max(left as i64) as u32;
    let bottom = (y as i64 + height as i64)
        .clamp(0, image.height() as i64)
        .max(top as i64) as u32;

    for pixel_y in top..bottom {
        for pixel_x in left..right {
            image.put_pixel(pixel_x, pixel_y, color);
        }
    }
}

pub fn fill_horizontal_line(image: &mut RgbImage, x0: i32, x1: i32, y: i32, color: Rgb<u8>) {
    if y < 0 || y >= image.height() as i32 {
        return;
    }
    let left = x0.min(x1).max(0) as u32;
    let right = x0.max(x1).min(image.width() as i32 - 1);
    if right < 0 || left > right as u32 {
        return;
    }
    for pixel_x in left..=right as u32 {
        image.put_pixel(pixel_x, y as u32, color);
    }
}

pub fn fill_circle(image: &mut RgbImage, center: (i32, i32), radius: i32, color: Rgb<u8>) {
    let mut x = 0;
    let mut y = radius;
    let mut p = 1 - radius;
    let (x0, y0) = center;

    while x <= y {
        fill_horizontal_line(image, x0 - x, x0 + x, y0 + y, color);
        fill_horizontal_line(image, x0 - y, x0 + y, y0 + x, color);
        fill_horizontal_line(image, x0 - x, x0 + x, y0 - y, color);
        fill_horizontal_line(image, x0 - y, x0 + y, y0 - x, color);

        x += 1;
        if p < 0 {
            p += 2 * x + 1;
        } else {
            y -= 1;
            p += 2 * (x - y) + 1;
        }
    }
}

pub fn blend_channel(left: u8, right: u8, left_weight: f32, right_weight: f32) -> u8 {
    let value = left as f32 * left_weight + right as f32 * right_weight;
    if value < u8::MAX as f32 {
        if value > u8::MIN as f32 {
            value as u8
        } else {
            u8::MIN
        }
    } else {
        u8::MAX
    }
}

pub fn blend_rgb(left: Rgb<u8>, right: Rgb<u8>, left_weight: f32, right_weight: f32) -> Rgb<u8> {
    Rgb([
        blend_channel(left[0], right[0], left_weight, right_weight),
        blend_channel(left[1], right[1], left_weight, right_weight),
        blend_channel(left[2], right[2], left_weight, right_weight),
    ])
}

pub fn layout_lb_glyphs(
    scale: PxScale,
    font: &impl Font,
    text: &str,
    mut visit: impl FnMut(OutlinedGlyph, GlyphRect),
) {
    let font = font.as_scaled(scale);
    let mut last: Option<GlyphId> = None;
    let mut width = 0.0;

    for character in text.chars() {
        let glyph_id = font.glyph_id(character);
        let glyph = glyph_id.with_scale_and_position(scale, point(width, font.ascent()));
        width += font.h_advance(glyph_id);
        if let Some(outlined) = font.outline_glyph(glyph) {
            if let Some(last) = last {
                width += font.kern(glyph_id, last);
            }
            last = Some(glyph_id);
            let bounds = outlined.px_bounds();
            visit(outlined, bounds);
        }
    }
}

pub fn draw_lb_text(
    image: &mut RgbImage,
    font: &FontArc,
    size: f32,
    x: i32,
    y: i32,
    text: impl AsRef<str>,
    color: Rgb<u8>,
) {
    let image_width = image.width() as i32;
    let image_height = image.height() as i32;

    layout_lb_glyphs(PxScale::from(size), font, text.as_ref(), |glyph, bounds| {
        glyph.draw(|glyph_x, glyph_y, value| {
            let image_x = glyph_x as i32 + x + bounds.min.x.round() as i32;
            let image_y = glyph_y as i32 + y + bounds.min.y.round() as i32;
            let value = value.clamp(0.0, 1.0);

            if (0..image_width).contains(&image_x) && (0..image_height).contains(&image_y) {
                let pixel = *image.get_pixel(image_x as u32, image_y as u32);
                image.put_pixel(
                    image_x as u32,
                    image_y as u32,
                    blend_rgb(pixel, color, 1.0 - value, value),
                );
            }
        });
    });
}

pub fn truncate_for_board(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut text = value
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    text.push_str("...");
    text
}

pub fn leaderboard_metric_column(metric: &str) -> &'static str {
    match metric {
        "winrate" => "winrate",
        "games" => "games",
        "mafia" => "mafia",
        "playtime" => "time",
        "rating" => "rating",
        _ => "record",
    }
}

pub fn render_leaderboard_image(stats_file: &stats::StatsFile, metric: &str) -> Option<Vec<u8>> {
    let entries = stats::leaderboard_entries(stats_file, metric, 10);
    if entries.is_empty() {
        return None;
    }

    const IMAGE_WIDTH: u32 = 1280;
    const TOP_PADDING: i32 = 40;
    const SIDE_PADDING: i32 = 48;
    const HEADER_HEIGHT: i32 = 150;
    const ROW_HEIGHT: i32 = 78;
    const BOTTOM_PADDING: i32 = 44;

    let height =
        (TOP_PADDING + HEADER_HEIGHT + ROW_HEIGHT * entries.len() as i32 + BOTTOM_PADDING) as u32;
    let mut image = RgbImage::from_pixel(IMAGE_WIDTH, height, image_color("#111318"));
    let font = FontArc::try_from_slice(include_bytes!("../MalangmalangR.ttf")).ok()?;

    let text = image_color("#f5f7fb");
    let muted = image_color("#aeb6c8");
    let accent = image_color("#ffd166");
    let panel = image_color("#1d2028");
    let row_dark = image_color("#242832");
    let row_light = image_color("#292e3a");

    draw_lb_text(
        &mut image,
        &font,
        44.0,
        SIDE_PADDING,
        TOP_PADDING,
        "마피아 리더보드",
        text,
    );
    draw_lb_text(
        &mut image,
        &font,
        24.0,
        SIDE_PADDING,
        TOP_PADDING + 58,
        "게임 종료 후 기록된 전적 기준",
        muted,
    );
    fill_rect(
        &mut image,
        IMAGE_WIDTH as i32 - SIDE_PADDING - 230,
        TOP_PADDING + 10,
        210,
        38,
        image_color("#374151"),
    );
    draw_lb_text(
        &mut image,
        &font,
        24.0,
        IMAGE_WIDTH as i32 - SIDE_PADDING - 214,
        TOP_PADDING + 16,
        format!("기준: {}", stats::leaderboard_metric_name(metric)),
        text,
    );

    let panel_top = TOP_PADDING + 116;
    let panel_bottom = height as i32 - BOTTOM_PADDING + 8;
    fill_rect(
        &mut image,
        SIDE_PADDING,
        panel_top,
        IMAGE_WIDTH - (SIDE_PADDING as u32 * 2),
        (panel_bottom - panel_top) as u32,
        panel,
    );

    let columns = HashMap::from([
        ("rank", SIDE_PADDING + 32),
        ("name", SIDE_PADDING + 110),
        ("record", SIDE_PADDING + 410),
        ("games", SIDE_PADDING + 555),
        ("winrate", SIDE_PADDING + 665),
        ("mafia", SIDE_PADDING + 800),
        ("time", SIDE_PADDING + 930),
        ("rating", SIDE_PADDING + 1085),
    ]);
    let selected_column = leaderboard_metric_column(metric);
    let header_y = panel_top + 24;
    for (key, label) in [
        ("rank", "#"),
        ("name", "이름"),
        ("record", "승패"),
        ("games", "판수"),
        ("winrate", "승률"),
        ("mafia", "마피아"),
        ("time", "시간"),
        ("rating", "레이팅"),
    ] {
        draw_lb_text(
            &mut image,
            &font,
            21.0,
            columns[key],
            header_y,
            label,
            if key == selected_column {
                accent
            } else {
                muted
            },
        );
    }

    let row_start_y = panel_top + 62;
    for (index, (_user_id, entry)) in entries.iter().enumerate() {
        let rank = index + 1;
        let y = row_start_y + index as i32 * ROW_HEIGHT;
        let row_fill = if rank % 2 == 1 { row_dark } else { row_light };
        fill_rect(
            &mut image,
            SIDE_PADDING + 18,
            y,
            IMAGE_WIDTH - ((SIDE_PADDING + 18) as u32 * 2),
            (ROW_HEIGHT - 10) as u32,
            row_fill,
        );
        let medal = match rank {
            1 => image_color("#f6c945"),
            2 => image_color("#c4ccd8"),
            3 => image_color("#c58b5b"),
            _ => image_color("#3b4252"),
        };
        fill_circle(&mut image, (columns["rank"] + 17, y + 36), 20, medal);
        draw_lb_text(
            &mut image,
            &font,
            24.0,
            columns["rank"] + if rank < 10 { 9 } else { 3 },
            y + 22,
            rank.to_string(),
            if rank <= 3 {
                image_color("#111318")
            } else {
                text
            },
        );

        let name = if entry.name.is_empty() {
            "알 수 없음".to_string()
        } else {
            truncate_for_board(&entry.name, 13)
        };
        let values = [
            ("name", name),
            ("record", format!("{}승 {}패", entry.wins, entry.losses)),
            ("games", format!("{}판", entry.games)),
            ("winrate", stats::win_rate_text(entry.wins, entry.games)),
            ("mafia", format!("{}회", entry.mafia_team_games)),
            ("time", stats::play_duration_text(entry.play_seconds)),
            ("rating", format!("{}점", entry.rating)),
        ];
        for (key, value) in values {
            draw_lb_text(
                &mut image,
                &font,
                if key == "name" { 27.0 } else { 23.0 },
                columns[key],
                y + if key == "name" { 18 } else { 21 },
                value,
                if key == selected_column { accent } else { text },
            );
        }
    }
    draw_lb_text(
        &mut image,
        &font,
        18.0,
        SIDE_PADDING + 18,
        height as i32 - 30,
        "마피아 게임 진행 메시지",
        muted,
    );

    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut bytes, ImageFormat::Png)
        .ok()?;
    Some(bytes.into_inner())
}

#[poise::command(
    slash_command,
    rename = "리더보드",
    description_localized("ko", "마피아 게임 전적 순위를 확인합니다.")
)]
pub async fn show_leaderboard(
    ctx: Context<'_>,
    #[description = "정렬 기준"] 기준: Option<LeaderboardMetric>,
) -> Result<(), Error> {
    let metric = 기준.map_or("wins", LeaderboardMetric::value);
    let stats_file = ctx.data().stats.read().await;
    if let Some(image) = render_leaderboard_image(&stats_file, metric) {
        ctx.send(
            poise::CreateReply::default().attachment(serenity::CreateAttachment::bytes(
                image,
                format!("mafia_leaderboard_{metric}.png"),
            )),
        )
        .await?;
        return Ok(());
    }
    let text = stats::leaderboard_text(&stats_file, metric);
    reply_embed(ctx, text, "리더보드", serenity::Colour::GOLD, false).await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "리더보드초기화",
    description_localized("ko", "마피아 게임 전적과 리더보드를 초기화합니다.")
)]
pub async fn reset_leaderboard(ctx: Context<'_>) -> Result<(), Error> {
    if !require_manager(ctx).await? {
        return Ok(());
    }
    let mut stats_file = ctx.data().stats.write().await;
    *stats_file = stats::StatsFile::default();
    stats::save_stats(&*ctx.data().stats_path, &stats_file)?;
    reply_embed(
        ctx,
        "리더보드와 개인 전적을 초기화했습니다.",
        "리더보드",
        serenity::Colour::DARK_GREEN,
        false,
    )
    .await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "마피아설정",
    description_localized("ko", "마피아 게임 기본 설정을 변경합니다.")
)]
#[allow(clippy::too_many_arguments)]
pub async fn configure_game(
    ctx: Context<'_>,
    #[description = "마피아 수"] mafia: Option<u32>,
    #[description = "의사 수"] doctor: Option<u32>,
    #[description = "경찰 수"] police: Option<u32>,
    #[description = "시민 특수룰 수"] citizen_special: Option<u32>,
    #[description = "마피아 특수룰 수"] mafia_special: Option<u32>,
    #[description = "중립 특수룰 수"] neutral_special: Option<u32>,
    #[description = "낮 채팅 슬로우모드 초. 기본 3초"] slowmode: Option<u64>,
    #[description = "사망 시 직업 공개 여부"] death_role_reveal: Option<bool>,
    #[description = "낮에 경찰 조사 성공 여부 공개 여부"] police_status_reveal: Option<bool>,
    #[description = "아침 생존 마피아 수 공개 여부"] mafia_count_reveal: Option<bool>,
    #[description = "사립탐정 활성화 여부"] detective: Option<bool>,
    #[description = "영매 활성화 여부"] shaman: Option<bool>,
    #[description = "도굴꾼 활성화 여부"] graverobber: Option<bool>,
    #[description = "스파이 활성화 여부"] spy: Option<bool>,
    #[description = "청부업자 활성화 여부"] contractor: Option<bool>,
    #[description = "마녀 활성화 여부"] witch: Option<bool>,
    #[description = "과학자 활성화 여부"] scientist: Option<bool>,
    #[description = "대부 활성화 여부"] godfather: Option<bool>,
    #[description = "조커 활성화 여부"] joker: Option<bool>,
    #[description = "정치인 활성화 여부"] politician: Option<bool>,
    #[description = "판사 활성화 여부"] judge: Option<bool>,
    #[description = "기자 활성화 여부"] reporter: Option<bool>,
    #[description = "해커 활성화 여부"] hacker: Option<bool>,
    #[description = "테러리스트 활성화 여부"] terrorist: Option<bool>,
    #[description = "군인 활성화 여부"] soldier: Option<bool>,
) -> Result<(), Error> {
    if !require_manager(ctx).await? {
        return Ok(());
    }
    let mut config_write = ctx.data().config.write().await;
    let previous = config_write.clone();
    if let Some(value) = mafia {
        if value < 1 {
            reply_embed(
                ctx,
                "마피아는 최소 1명이어야 합니다.",
                "설정 오류",
                serenity::Colour::RED,
                true,
            )
            .await?;
            return Ok(());
        }
        config_write.default_mafia_count = value;
    }
    if let Some(value) = doctor {
        config_write.default_doctor_count = value;
    }
    if let Some(value) = police {
        config_write.default_police_count = value;
    }
    if let Some(value) = citizen_special {
        config_write.citizen_special_count = value;
    }
    if let Some(value) = mafia_special {
        config_write.mafia_special_count = value;
    }
    if let Some(value) = neutral_special {
        config_write.neutral_special_count = value;
    }
    if let Some(value) = slowmode {
        config_write.chat_slowmode_seconds = value;
    }
    if let Some(value) = death_role_reveal {
        config_write.reveal_death_roles = value;
    }
    if let Some(value) = police_status_reveal {
        config_write.reveal_public_police_status = value;
    }
    if let Some(value) = mafia_count_reveal {
        config_write.reveal_morning_mafia_count = value;
    }
    if let Some(value) = detective {
        config_write.enable_detective = value;
    }
    if let Some(value) = shaman {
        config_write.enable_shaman = value;
    }
    if let Some(value) = graverobber {
        config_write.enable_graverobber = value;
    }
    if let Some(value) = spy {
        config_write.enable_spy = value;
    }
    if let Some(value) = contractor {
        config_write.enable_contractor = value;
    }
    if let Some(value) = witch {
        config_write.enable_witch = value;
    }
    if let Some(value) = scientist {
        config_write.enable_scientist = value;
    }
    if let Some(value) = godfather {
        config_write.enable_godfather = value;
    }
    if let Some(value) = joker {
        config_write.enable_joker = value;
    }
    if let Some(value) = politician {
        config_write.enable_politician = value;
    }
    if let Some(value) = judge {
        config_write.enable_judge = value;
    }
    if let Some(value) = reporter {
        config_write.enable_reporter = value;
    }
    if let Some(value) = hacker {
        config_write.enable_hacker = value;
    }
    if let Some(value) = terrorist {
        config_write.enable_terrorist = value;
    }
    if let Some(value) = soldier {
        config_write.enable_soldier = value;
    }
    let validation = choose_special_roles(&config_write)
        .and_then(|special_roles| selected_role_counts(&config_write, &special_roles))
        .map(|role_counts| {
            let minimum_players = minimum_player_count(&role_counts);
            let max_players = effective_max_player_count(&config_write);
            (minimum_players, max_players)
        });
    match validation {
        Ok((minimum_players, max_players)) if max_players < minimum_players => {
            *config_write = previous;
            reply_embed(
                ctx,
                format!("현재 설정의 최소 시작 인원은 {minimum_players}명이라 최대 인원 {max_players}명으로 시작할 수 없습니다."),
                "설정 오류",
                serenity::Colour::RED,
                true,
            )
            .await?;
            return Ok(());
        }
        Err(error) => {
            *config_write = previous;
            reply_embed(
                ctx,
                error.to_string(),
                "설정 오류",
                serenity::Colour::RED,
                true,
            )
            .await?;
            return Ok(());
        }
        _ => {}
    }
    config::save_config(&*ctx.data().config_path, &config_write)?;
    let text = current_settings_text(&config_write, "마피아 설정을 저장했습니다.");
    drop(config_write);
    reply_embed(
        ctx,
        text,
        "마피아 설정",
        serenity::Colour::DARK_GREEN,
        false,
    )
    .await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "마피아인원설정",
    description_localized("ko", "마피아 게임 모집 최대 인원을 설정합니다.")
)]
pub async fn configure_player_limit(
    ctx: Context<'_>,
    #[description = "최대 참가 인원. 0은 제한 없음(봇 최대 24명)"] max_players: u32,
) -> Result<(), Error> {
    if !require_manager(ctx).await? {
        return Ok(());
    }
    if max_players as usize > MAX_GAME_PLAYERS {
        reply_embed(
            ctx,
            format!("최대 인원은 {MAX_GAME_PLAYERS}명 이하로 설정해야 합니다."),
            "설정 오류",
            serenity::Colour::RED,
            true,
        )
        .await?;
        return Ok(());
    }
    let mut config_write = ctx.data().config.write().await;
    config_write.max_player_count = max_players;
    config::save_config(&*ctx.data().config_path, &config_write)?;
    let text = current_settings_text(&config_write, "마피아 인원 설정을 저장했습니다.");
    drop(config_write);
    reply_embed(
        ctx,
        text,
        "마피아 설정",
        serenity::Colour::DARK_GREEN,
        false,
    )
    .await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "마피아익명설정",
    description_localized("ko", "마피아 게임 익명 채팅 사용 여부를 설정합니다.")
)]
pub async fn configure_anonymous_mode(
    ctx: Context<'_>,
    #[description = "익명 채팅 사용 여부"] enabled: bool,
    #[description = "익명 이름을 동물로 할지 숫자로 할지 선택합니다."] 이름방식: Option<
        AnonymousNameMode,
    >,
) -> Result<(), Error> {
    if !require_manager(ctx).await? {
        return Ok(());
    }
    let mut config_write = ctx.data().config.write().await;
    config_write.anonymous_mode = enabled;
    if let Some(name_mode) = 이름방식 {
        config_write.anonymous_name_mode = name_mode.value().to_string();
    }
    config::save_config(&*ctx.data().config_path, &config_write)?;
    let text = current_settings_text(&config_write, "마피아 익명 설정을 저장했습니다.");
    drop(config_write);
    reply_embed(
        ctx,
        text,
        "마피아 설정",
        serenity::Colour::DARK_GREEN,
        false,
    )
    .await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "마피아웹설정",
    description_localized(
        "ko",
        "브라우저에서 게임 설정을 편집할 수 있는 1회용 링크를 발급합니다. (관리자 전용)"
    )
)]
pub async fn web_configure_game(ctx: Context<'_>) -> Result<(), Error> {
    if !require_manager(ctx).await? {
        return Ok(());
    }
    let Some(guild_id) = ctx.guild_id() else {
        reply_embed(
            ctx,
            "서버에서만 사용할 수 있습니다.",
            "웹 설정",
            serenity::Colour::RED,
            true,
        )
        .await?;
        return Ok(());
    };
    let user = ctx.author();
    let token = web_settings::issue_session(
        &ctx.data().web_sessions,
        guild_id.get(),
        user.id.get(),
        user.name.clone(),
    );
    let url = format!(
        "{}{}/{}",
        ctx.data().web_base_url.trim_end_matches('/'),
        web_settings::settings_path(),
        token
    );
    let minutes = web_settings::session_ttl_minutes();
    reply_embed(
        ctx,
        format!(
            "아래 링크에서 마피아 게임 설정을 편집할 수 있습니다.\n{url}\n\n⚠️ 이 링크는 **{}** 님만 사용할 수 있고, **{minutes}분 동안 1회**만 유효합니다. 다른 사람과 공유하지 마세요.",
            user.name
        ),
        "웹 설정 링크 발급",
        serenity::Colour::DARK_GREEN,
        true,
    )
    .await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "마피아추가설정",
    description_localized("ko", "추가 역할 묶음을 설정합니다.")
)]
#[allow(clippy::too_many_arguments)]
pub async fn configure_extra_roles(
    ctx: Context<'_>,
    nurse: Option<bool>,
    lover: Option<bool>,
    priest: Option<bool>,
    madam: Option<bool>,
    gangster: Option<bool>,
    prophet: Option<bool>,
    psychologist: Option<bool>,
    thief: Option<bool>,
    cult_team: Option<bool>,
) -> Result<(), Error> {
    if !require_manager(ctx).await? {
        return Ok(());
    }
    let mut config_write = ctx.data().config.write().await;
    if let Some(v) = nurse {
        config_write.enable_nurse = v;
    }
    if let Some(v) = lover {
        config_write.enable_lover = v;
    }
    if let Some(v) = priest {
        config_write.enable_priest = v;
    }
    if let Some(v) = madam {
        config_write.enable_madam = v;
    }
    if let Some(v) = gangster {
        config_write.enable_gangster = v;
    }
    if let Some(v) = prophet {
        config_write.enable_prophet = v;
    }
    if let Some(v) = psychologist {
        config_write.enable_psychologist = v;
    }
    if let Some(v) = thief {
        config_write.enable_thief = v;
    }
    if let Some(v) = cult_team {
        config_write.enable_cult_team = v;
    }
    config::save_config(&*ctx.data().config_path, &config_write)?;
    let text = current_settings_text(&config_write, "마피아 추가 설정을 저장했습니다.");
    drop(config_write);
    reply_embed(
        ctx,
        text,
        "마피아 설정",
        serenity::Colour::DARK_GREEN,
        false,
    )
    .await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "마피아수사설정",
    description_localized("ko", "수사직 후보를 설정합니다.")
)]
pub async fn configure_investigation_role(
    ctx: Context<'_>,
    agent: Option<bool>,
    vigilante: Option<bool>,
) -> Result<(), Error> {
    if !require_manager(ctx).await? {
        return Ok(());
    }
    let mut config_write = ctx.data().config.write().await;
    if let Some(v) = agent {
        config_write.use_agent = v;
    }
    if let Some(v) = vigilante {
        config_write.use_vigilante = v;
    }
    config::save_config(&*ctx.data().config_path, &config_write)?;
    let text = current_settings_text(&config_write, "마피아 수사 설정을 저장했습니다.");
    drop(config_write);
    reply_embed(
        ctx,
        text,
        "마피아 설정",
        serenity::Colour::DARK_GREEN,
        false,
    )
    .await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "마피아비활성화",
    description_localized("ko", "마피아 게임 시작을 비활성화합니다.")
)]
pub async fn disable_mafia_game(ctx: Context<'_>) -> Result<(), Error> {
    set_game_enabled(ctx, false).await
}

#[poise::command(
    slash_command,
    rename = "마피아활성화",
    description_localized("ko", "마피아 게임 시작을 활성화합니다.")
)]
pub async fn enable_mafia_game(ctx: Context<'_>) -> Result<(), Error> {
    set_game_enabled(ctx, true).await
}

pub async fn set_game_enabled(ctx: Context<'_>, enabled: bool) -> Result<(), Error> {
    if !require_manager(ctx).await? {
        return Ok(());
    }
    let mut config_write = ctx.data().config.write().await;
    config_write.game_enabled = enabled;
    config::save_config(&*ctx.data().config_path, &config_write)?;
    drop(config_write);
    reply_embed(
        ctx,
        if enabled {
            "마피아 게임을 활성화했습니다. 이제 새 게임을 시작할 수 있습니다."
        } else {
            "마피아 게임을 비활성화했습니다. 새 게임을 시작할 수 없습니다."
        },
        "마피아 게임",
        serenity::Colour::DARK_GREEN,
        false,
    )
    .await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "블랙리스트추가",
    description_localized("ko", "마피아 게임 참가 블랙리스트에 유저를 추가합니다.")
)]
pub async fn add_to_blacklist(
    ctx: Context<'_>,
    #[description = "블랙리스트에 추가할 유저"] 유저: serenity::User,
) -> Result<(), Error> {
    if !require_manager(ctx).await? {
        return Ok(());
    }
    let mut config_write = ctx.data().config.write().await;
    let id = 유저.id.get();
    let changed = !config_write.blacklist_user_ids.contains(&id);
    if changed {
        config_write.blacklist_user_ids.push(id);
        config_write.blacklist_user_ids.sort_unstable();
    }
    config::save_config(&*ctx.data().config_path, &config_write)?;
    drop(config_write);
    reply_embed(
        ctx,
        if changed {
            format!(
                "{} 님을 블랙리스트에 추가했습니다. 이제 게임에 참가할 수 없습니다.",
                유저.name
            )
        } else {
            format!("{} 님은 이미 블랙리스트에 있습니다.", 유저.name)
        },
        "블랙리스트",
        serenity::Colour::DARK_GREEN,
        false,
    )
    .await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "블랙리스트해제",
    description_localized("ko", "마피아 게임 참가 블랙리스트에서 유저를 제거합니다.")
)]
pub async fn remove_from_blacklist(
    ctx: Context<'_>,
    #[description = "블랙리스트에서 해제할 유저"] 유저: serenity::User,
) -> Result<(), Error> {
    if !require_manager(ctx).await? {
        return Ok(());
    }
    let mut config_write = ctx.data().config.write().await;
    let id = 유저.id.get();
    let before = config_write.blacklist_user_ids.len();
    config_write
        .blacklist_user_ids
        .retain(|user_id| *user_id != id);
    let changed = config_write.blacklist_user_ids.len() != before;
    config::save_config(&*ctx.data().config_path, &config_write)?;
    drop(config_write);
    reply_embed(
        ctx,
        if changed {
            format!(
                "{} 님을 블랙리스트에서 해제했습니다. 이제 게임에 참가할 수 있습니다.",
                유저.name
            )
        } else {
            format!("{} 님은 블랙리스트에 없습니다.", 유저.name)
        },
        "블랙리스트",
        serenity::Colour::DARK_GREEN,
        false,
    )
    .await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "블랙리스트목록",
    description_localized("ko", "마피아 게임 참가 블랙리스트 목록을 확인합니다.")
)]
pub async fn show_blacklist(ctx: Context<'_>) -> Result<(), Error> {
    if !require_manager(ctx).await? {
        return Ok(());
    }
    let config_read = ctx.data().config.read().await;
    let text = if config_read.blacklist_user_ids.is_empty() {
        "블랙리스트가 비어 있습니다.".to_string()
    } else {
        config_read
            .blacklist_user_ids
            .iter()
            .take(50)
            .enumerate()
            .map(|(i, id)| format!("{}. `{id}`", i + 1))
            .collect::<Vec<_>>()
            .join("\n")
    };
    drop(config_read);
    reply_embed(ctx, text, "블랙리스트", serenity::Colour::GOLD, true).await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "직업정보",
    description_localized("ko", "특정 직업의 설명을 확인합니다.")
)]
pub async fn show_role_info(
    ctx: Context<'_>,
    #[description = "설명을 볼 직업 이름"] 직업명: String,
) -> Result<(), Error> {
    let role = find_role_by_name(&직업명);
    if let Some(role) = role {
        reply_embed(
            ctx,
            format!("{}\n{}", role.value(), role_short_guide(role)),
            "직업정보",
            serenity::Colour::DARK_GREEN,
            false,
        )
        .await?;
    } else {
        reply_embed(
            ctx,
            "직업을 찾을 수 없습니다. 정확한 직업명을 입력하세요.",
            "직업정보",
            serenity::Colour::RED,
            true,
        )
        .await?;
    }
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "역할설명",
    description_localized("ko", "마피아 게임 전체 역할 설명을 공지용 임베드로 보냅니다.")
)]
pub async fn show_role_descriptions(ctx: Context<'_>) -> Result<(), Error> {
    let mut lines = Vec::new();
    for role in [
        Role::Mafia,
        Role::Police,
        Role::Agent,
        Role::Vigilante,
        Role::Doctor,
        Role::Nurse,
        Role::Gangster,
        Role::Prophet,
        Role::Psychologist,
        Role::Detective,
        Role::Shaman,
        Role::Priest,
        Role::Graverobber,
        Role::Politician,
        Role::Judge,
        Role::Reporter,
        Role::Hacker,
        Role::Terrorist,
        Role::Lover,
        Role::Soldier,
        Role::Spy,
        Role::Contractor,
        Role::Thief,
        Role::Witch,
        Role::Scientist,
        Role::Madam,
        Role::Godfather,
        Role::CultLeader,
        Role::Fanatic,
        Role::Joker,
        Role::Citizen,
    ] {
        lines.push(format!("**{}** - {}", role.value(), role_short_guide(role)));
    }
    reply_embed(
        ctx,
        lines.join("\n"),
        "역할 설명",
        serenity::Colour::GOLD,
        false,
    )
    .await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "마피아능력",
    description_localized("ko", "배정받은 역할과 능력 설명을 다시 확인합니다.")
)]
pub async fn show_abilities(ctx: Context<'_>) -> Result<(), Error> {
    let Some(guild_id) = ctx.guild_id() else {
        reply_embed(
            ctx,
            "서버에서만 사용할 수 있습니다.",
            "능력 설명",
            serenity::Colour::RED,
            true,
        )
        .await?;
        return Ok(());
    };
    let Some(running) = ctx.data().games.get(&guild_id).map(|entry| entry.clone()) else {
        reply_embed(
            ctx,
            "진행 중인 게임이 없습니다.",
            "능력 설명",
            serenity::Colour::RED,
            true,
        )
        .await?;
        return Ok(());
    };
    let running_read = running.read().await;
    let Some(player) = running_read.game.get_player(ctx.author().id.get()) else {
        reply_embed(
            ctx,
            "현재 게임 참가자만 능력 설명을 확인할 수 있습니다.",
            "능력 설명",
            serenity::Colour::RED,
            true,
        )
        .await?;
        return Ok(());
    };
    reply_embed(
        ctx,
        role_message(&running_read.game, player),
        "능력 설명",
        serenity::Colour::GOLD,
        true,
    )
    .await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "용어정보",
    description_localized("ko", "마피아 게임 용어 하나를 확인합니다.")
)]
pub async fn show_term_info(
    ctx: Context<'_>,
    #[description = "설명을 볼 용어"] 용어: String,
) -> Result<(), Error> {
    let Some(term) = find_term_by_name(&용어) else {
        reply_embed(
            ctx,
            "용어를 찾을 수 없습니다. 정확한 용어를 입력하세요.",
            "용어정보",
            serenity::Colour::RED,
            true,
        )
        .await?;
        return Ok(());
    };
    reply_embed(
        ctx,
        format!("분류: {}\n\n{}", term.category, term_field_value(&term)),
        &format!("용어정보 - {}", term.names[0]),
        serenity::Colour::DARK_GREEN,
        false,
    )
    .await?;
    Ok(())
}

#[poise::command(
    slash_command,
    rename = "용어설명",
    description_localized("ko", "마피아 게임 용어 설명을 공지용 임베드로 보냅니다.")
)]
pub async fn show_term_descriptions(ctx: Context<'_>) -> Result<(), Error> {
    for (index, (title, body)) in term_guide_pages().into_iter().enumerate() {
        if index == 0 {
            reply_embed(ctx, body, &title, serenity::Colour::GOLD, false).await?;
        } else {
            send_channel_embed(
                &ctx.serenity_context().http,
                ctx.channel_id(),
                body,
                &title,
                serenity::Colour::GOLD,
                vec![],
            )
            .await?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct TermEntry {
    category: String,
    names: Vec<String>,
    meaning: String,
    example: String,
}

pub fn find_term_by_name(name: &str) -> Option<TermEntry> {
    let query = name.trim().to_lowercase();
    if query.is_empty() {
        return None;
    }
    let terms = mafia_term_entries();
    for term in &terms {
        if term.names.iter().any(|alias| alias.to_lowercase() == query) {
            return Some(term.clone());
        }
    }
    let matches = terms
        .into_iter()
        .filter(|term| {
            term.names
                .iter()
                .any(|alias| alias.to_lowercase().contains(&query))
        })
        .collect::<Vec<_>>();
    if matches.len() == 1 {
        matches.into_iter().next()
    } else {
        None
    }
}

pub fn term_field_value(term: &TermEntry) -> String {
    let mut lines = vec![term.meaning.clone()];
    if term.names.len() > 1 {
        lines.push(format!("같은 말: {}", term.names[1..].join(", ")));
    }
    if !term.example.is_empty() {
        lines.push(format!("예시: {}", term.example));
    }
    lines.join("\n")
}

pub fn term_guide_pages() -> Vec<(String, String)> {
    let mut pages = Vec::new();
    let mut grouped: Vec<(String, Vec<TermEntry>)> = Vec::new();
    for term in mafia_term_entries() {
        if let Some((_category, terms)) = grouped
            .iter_mut()
            .find(|(category, _terms)| *category == term.category)
        {
            terms.push(term);
        } else {
            grouped.push((term.category.clone(), vec![term]));
        }
    }
    for (category, terms) in grouped {
        let mut body =
            "마피아42 용어 문서를 참고해 이 봇 진행에 맞게 짧게 정리한 용어집입니다.".to_string();
        let mut page_index = 1;
        for term in terms {
            let entry = format!("\n\n**{}**\n{}", term.names[0], term_field_value(&term));
            if body.len() + entry.len() > 3600 {
                let title = if page_index == 1 {
                    format!("용어 설명 - {category}")
                } else {
                    format!("용어 설명 - {category} {page_index}")
                };
                pages.push((title, body));
                page_index += 1;
                body = "마피아42 용어 문서를 참고해 이 봇 진행에 맞게 짧게 정리한 용어집입니다."
                    .to_string();
            }
            body.push_str(&entry);
        }
        let title = if page_index == 1 {
            format!("용어 설명 - {category}")
        } else {
            format!("용어 설명 - {category} {page_index}")
        };
        pages.push((title, body));
    }
    pages
}

pub fn mafia_term_entries() -> Vec<TermEntry> {
    let mut terms = Vec::new();
    let mut in_section = false;
    for line in include_str!("../role_data.py").lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("MAFIA_TERM_ENTRIES") {
            in_section = true;
            continue;
        }
        if !in_section {
            continue;
        }
        if trimmed == ")" {
            break;
        }
        if !trimmed.starts_with("(\"") {
            continue;
        }
        let strings = extract_python_strings(trimmed);
        if strings.len() < 4 {
            continue;
        }
        let category = strings[0].clone();
        let meaning = strings[strings.len() - 2].clone();
        let example = strings[strings.len() - 1].clone();
        let names = strings[1..strings.len() - 2].to_vec();
        if !names.is_empty() {
            terms.push(TermEntry {
                category,
                names,
                meaning,
                example,
            });
        }
    }
    terms
}

pub fn extract_python_strings(line: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut escaped = false;
    for ch in line.chars() {
        if !in_string {
            if ch == '"' {
                in_string = true;
                current.clear();
            }
            continue;
        }
        if escaped {
            current.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            in_string = false;
            values.push(current.clone());
        } else {
            current.push(ch);
        }
    }
    values
}

pub fn find_role_by_name(name: &str) -> Option<Role> {
    let query = name.trim();
    [
        Role::Mafia,
        Role::Doctor,
        Role::Nurse,
        Role::Police,
        Role::Agent,
        Role::Vigilante,
        Role::Reporter,
        Role::Hacker,
        Role::Detective,
        Role::Shaman,
        Role::Priest,
        Role::Soldier,
        Role::Gangster,
        Role::Prophet,
        Role::Psychologist,
        Role::Spy,
        Role::Contractor,
        Role::Thief,
        Role::Witch,
        Role::Scientist,
        Role::Madam,
        Role::Graverobber,
        Role::Godfather,
        Role::Joker,
        Role::Politician,
        Role::Judge,
        Role::Terrorist,
        Role::Lover,
        Role::CultLeader,
        Role::Fanatic,
        Role::Citizen,
    ]
    .into_iter()
    .find(|role| role.value() == query)
}

#[derive(Clone, Copy)]
enum AnonymousMessageKind {
    General { owner_id: u64 },
    Dead { owner_id: u64 },
    Shaman { owner_id: u64 },
    Role { owner_id: u64, role: Role },
}

pub fn anonymous_message_body(message: &serenity::Message) -> String {
    let mut parts = Vec::new();
    let content = message.content.trim();
    if !content.is_empty() {
        parts.push(content.to_string());
    }
    parts.extend(
        message
            .attachments
            .iter()
            .map(|attachment| attachment.url.clone()),
    );
    if parts.is_empty() {
        "(내용 없음)".to_string()
    } else {
        parts.join("\n")
    }
}

pub fn anonymous_avatar_url(author_label: &str) -> Option<String> {
    if let Some(number) = author_label
        .strip_suffix("번")
        .and_then(|value| value.parse::<usize>().ok())
    {
        let color = NUMBER_AVATAR_COLORS[(number.saturating_sub(1)) % NUMBER_AVATAR_COLORS.len()];
        return Some(format!(
            "https://dummyimage.com/128x128/{color}/ffffff.png&text={number}"
        ));
    }
    animal_emoji_code(author_label).map(|code| {
        format!("https://cdn.jsdelivr.net/gh/twitter/twemoji@14.0.2/assets/72x72/{code}.png")
    })
}

pub fn no_mentions() -> serenity::CreateAllowedMentions {
    serenity::CreateAllowedMentions::new()
        .all_users(false)
        .all_roles(false)
        .everyone(false)
        .replied_user(false)
}

pub async fn anonymous_webhook(
    ctx: &serenity::Context,
    running: &Arc<RwLock<RunningGame>>,
    channel_id: serenity::ChannelId,
) -> Option<serenity::Webhook> {
    let cached_url = running
        .read()
        .await
        .anonymous_webhook_urls
        .get(&channel_id)
        .cloned();
    if let Some(url) = cached_url
        && let Ok(webhook) = serenity::Webhook::from_url(&ctx.http, &url).await
    {
        return Some(webhook);
    }

    let webhook = channel_id
        .create_webhook(
            &ctx.http,
            serenity::CreateWebhook::new("Mafia Anonymous")
                .audit_log_reason("마피아 게임 익명 채팅 웹훅 생성"),
        )
        .await
        .ok()?;
    if let Some(url) = webhook.url.as_ref() {
        running
            .write()
            .await
            .anonymous_webhook_urls
            .insert(channel_id, url.expose_secret().to_string());
    }
    Some(webhook)
}

pub async fn send_anonymous_text(
    ctx: &serenity::Context,
    running: &Arc<RwLock<RunningGame>>,
    channel_id: serenity::ChannelId,
    author_label: &str,
    body: &str,
) {
    if let Some(webhook) = anonymous_webhook(ctx, running, channel_id).await {
        let username = author_label.chars().take(80).collect::<String>();
        let mut builder = serenity::ExecuteWebhook::new()
            .content(body)
            .username(username)
            .allowed_mentions(no_mentions());
        if let Some(avatar_url) = anonymous_avatar_url(author_label) {
            builder = builder.avatar_url(avatar_url);
        }
        if webhook.execute(&ctx.http, false, builder).await.is_ok() {
            return;
        }
    }
    let _ = channel_id
        .send_message(
            &ctx.http,
            serenity::CreateMessage::new()
                .content(format!("{author_label}: {body}"))
                .allowed_mentions(no_mentions()),
        )
        .await;
}

pub async fn send_webhook_text(
    ctx: &serenity::Context,
    running: &Arc<RwLock<RunningGame>>,
    channel_id: serenity::ChannelId,
    author_label: &str,
    avatar_url: Option<String>,
    body: &str,
) {
    if let Some(webhook) = anonymous_webhook(ctx, running, channel_id).await {
        let username = author_label.chars().take(80).collect::<String>();
        let mut builder = serenity::ExecuteWebhook::new()
            .content(body)
            .username(username)
            .allowed_mentions(no_mentions());
        if let Some(avatar_url) = avatar_url {
            builder = builder.avatar_url(avatar_url);
        }
        if webhook.execute(&ctx.http, false, builder).await.is_ok() {
            return;
        }
    }
    let _ = channel_id
        .send_message(
            &ctx.http,
            serenity::CreateMessage::new()
                .content(format!("{author_label}: {body}"))
                .allowed_mentions(no_mentions()),
        )
        .await;
}

pub fn message_author_display_name(message: &serenity::Message) -> String {
    message
        .member
        .as_ref()
        .and_then(|member| member.nick.clone())
        .or_else(|| message.author.global_name.clone())
        .unwrap_or_else(|| message.author.name.clone())
}

pub async fn send_message_author_webhook_text(
    ctx: &serenity::Context,
    running: &Arc<RwLock<RunningGame>>,
    channel_id: serenity::ChannelId,
    message: &serenity::Message,
    body: &str,
) {
    send_webhook_text(
        ctx,
        running,
        channel_id,
        &message_author_display_name(message),
        Some(message.author.face()),
        body,
    )
    .await;
}

pub fn can_use_anonymous_dead_chat(running: &RunningGame, player: &Player) -> bool {
    !player.alive && !running.game.purified_dead_ids.contains(&player.user_id)
}

pub fn can_use_anonymous_shaman_chat(running: &RunningGame, player: &Player) -> bool {
    if !player.alive {
        return !running.game.purified_dead_ids.contains(&player.user_id);
    }
    player.role == Role::Shaman
        && running.game.phase == Phase::Night
        && !running.game.is_frog(player)
        && !running.game.is_madam_seduced(player)
}

pub fn anonymous_dead_sender_label(running: &RunningGame, sender: &Player) -> String {
    if sender.alive && sender.role == Role::Shaman {
        "익명의 목소리".to_string()
    } else if running.anonymous_enabled {
        running
            .anonymous_aliases
            .get(&sender.user_id)
            .cloned()
            .unwrap_or_else(|| "익명".to_string())
    } else {
        sender.name.clone()
    }
}

pub async fn send_dead_chat_text(
    ctx: &serenity::Context,
    running: &Arc<RwLock<RunningGame>>,
    channel_id: serenity::ChannelId,
    sender: &Player,
    body: &str,
) {
    let (anonymous_enabled, guild_id, sender_label) = {
        let running_read = running.read().await;
        (
            running_read.anonymous_enabled,
            running_read.guild_id,
            anonymous_dead_sender_label(&running_read, sender),
        )
    };
    if anonymous_enabled {
        send_anonymous_text(ctx, running, channel_id, &sender_label, body).await;
        return;
    }
    if let Ok(member) = guild_id
        .member(ctx, serenity::UserId::new(sender.user_id))
        .await
    {
        send_webhook_text(
            ctx,
            running,
            channel_id,
            &display_name(&member),
            Some(member.face()),
            body,
        )
        .await;
        return;
    }
    send_anonymous_text(ctx, running, channel_id, &sender.name, body).await;
}

pub async fn mirror_role_chat_to_dead(
    ctx: &serenity::Context,
    data: &Data,
    running: &Arc<RwLock<RunningGame>>,
    message: &serenity::Message,
    role: Role,
    body: &str,
) {
    let Some(roles) = running_channel_roles(ctx, data, running).await else {
        return;
    };
    let (source_channel_id, viewers) = {
        let running_read = running.read().await;
        (
            running_read.channel_id,
            running_read
                .game
                .players
                .iter()
                .filter(|player| can_use_anonymous_dead_chat(&running_read, player))
                .cloned()
                .collect::<Vec<_>>(),
        )
    };
    if viewers.is_empty() {
        return;
    }
    let category = source_category(ctx, source_channel_id).await;
    let body = format!("[{}채팅] {body}", role.value());
    for viewer in viewers {
        let can_chat = {
            let running_read = running.read().await;
            running_read
                .game
                .get_player(viewer.user_id)
                .is_some_and(|player| can_use_anonymous_dead_chat(&running_read, player))
        };
        if let Some(channel_id) =
            ensure_anonymous_dead_input_channel(ctx, running, &viewer, roles, category, can_chat)
                .await
        {
            send_message_author_webhook_text(ctx, running, channel_id, message, &body).await;
        }
    }
}

pub async fn relay_anonymous_general_message(
    ctx: &serenity::Context,
    running: &Arc<RwLock<RunningGame>>,
    sender_id: u64,
    body: &str,
) {
    let (deliveries, log_channel, sender_alias) = {
        let running_read = running.read().await;
        let Some(sender) = running_read.game.get_player(sender_id) else {
            return;
        };
        let sender_alias = running_read
            .anonymous_aliases
            .get(&sender.user_id)
            .cloned()
            .unwrap_or_else(|| "익명".to_string());
        let deliveries = running_read
            .game
            .alive_players()
            .into_iter()
            .filter(|viewer| viewer.user_id != sender.user_id && !running_read.game.is_frog(viewer))
            .filter_map(|viewer| {
                running_read
                    .anonymous_input_channel_ids
                    .get(&viewer.user_id)
                    .copied()
            })
            .collect::<Vec<_>>();
        (deliveries, running_read.channel_id, sender_alias)
    };
    for channel_id in deliveries {
        send_anonymous_text(ctx, running, channel_id, &sender_alias, body).await;
    }
    send_anonymous_text(
        ctx,
        running,
        log_channel,
        "[익명 로그/일반]",
        &format!("{sender_alias} - {body}"),
    )
    .await;
}

pub async fn relay_anonymous_role_message(
    ctx: &serenity::Context,
    running: &Arc<RwLock<RunningGame>>,
    sender_id: u64,
    role: Role,
    body: &str,
) {
    let (deliveries, log_channel, sender_alias) = {
        let running_read = running.read().await;
        let Some(sender) = running_read.game.get_player(sender_id) else {
            return;
        };
        let sender_alias = running_read
            .anonymous_aliases
            .get(&sender.user_id)
            .cloned()
            .unwrap_or_else(|| "익명".to_string());
        let deliveries = anonymous_role_status_player_ids(&running_read, role)
            .into_iter()
            .filter(|viewer_id| *viewer_id != sender.user_id)
            .filter_map(|viewer_id| {
                let viewer = running_read.game.get_player(viewer_id)?;
                if !can_use_anonymous_role_chat(&running_read, viewer, role) {
                    return None;
                }
                running_read
                    .anonymous_role_input_channel_ids
                    .get(&(viewer_id, role))
                    .copied()
            })
            .collect::<Vec<_>>();
        (deliveries, running_read.channel_id, sender_alias)
    };
    for channel_id in deliveries {
        send_anonymous_text(ctx, running, channel_id, &sender_alias, body).await;
    }
    send_anonymous_text(
        ctx,
        running,
        log_channel,
        &format!("[익명 로그/{}]", role.value()),
        &format!("{sender_alias} - {body}"),
    )
    .await;
}

pub async fn relay_anonymous_dead_message(
    ctx: &serenity::Context,
    data: &Data,
    running: &Arc<RwLock<RunningGame>>,
    sender_id: u64,
    body: &str,
) {
    let Some(roles) = running_channel_roles(ctx, data, running).await else {
        return;
    };
    let (source_channel_id, sender, viewers) = {
        let running_read = running.read().await;
        let Some(sender) = running_read.game.get_player(sender_id) else {
            return;
        };
        (
            running_read.channel_id,
            sender.clone(),
            running_read
                .game
                .players
                .iter()
                .filter(|viewer| {
                    viewer.user_id != sender.user_id
                        && can_use_anonymous_dead_chat(&running_read, viewer)
                })
                .cloned()
                .collect::<Vec<_>>(),
        )
    };
    let category = source_category(ctx, source_channel_id).await;
    for viewer in viewers {
        let can_chat = {
            let running_read = running.read().await;
            running_read
                .game
                .get_player(viewer.user_id)
                .is_some_and(|player| can_use_anonymous_dead_chat(&running_read, player))
        };
        if let Some(channel_id) =
            ensure_anonymous_dead_input_channel(ctx, running, &viewer, roles, category, can_chat)
                .await
        {
            send_dead_chat_text(ctx, running, channel_id, &sender, body).await;
        }
    }
}

pub async fn relay_anonymous_shaman_message(
    ctx: &serenity::Context,
    running: &Arc<RwLock<RunningGame>>,
    sender_id: u64,
    body: &str,
) {
    let (deliveries, log_channel, sender_label) = {
        let running_read = running.read().await;
        let Some(sender) = running_read.game.get_player(sender_id) else {
            return;
        };
        let deliveries = running_read
            .game
            .players
            .iter()
            .filter(|viewer| {
                viewer.user_id != sender.user_id
                    && ((!viewer.alive
                        && !running_read
                            .game
                            .purified_dead_ids
                            .contains(&viewer.user_id))
                        || (viewer.alive
                            && viewer.role == Role::Shaman
                            && !running_read.game.is_frog(viewer)))
            })
            .filter_map(|viewer| {
                running_read
                    .anonymous_shaman_input_channel_ids
                    .get(&viewer.user_id)
                    .copied()
            })
            .collect::<Vec<_>>();
        (
            deliveries,
            running_read.shaman_channel_id,
            anonymous_dead_sender_label(&running_read, sender),
        )
    };
    for channel_id in deliveries {
        send_anonymous_text(ctx, running, channel_id, &sender_label, body).await;
    }
    if let Some(channel_id) = log_channel {
        send_anonymous_text(
            ctx,
            running,
            channel_id,
            "[익명 로그/영매]",
            &format!("{sender_label} - {body}"),
        )
        .await;
    }
}

pub async fn handle_anonymous_message(
    ctx: &serenity::Context,
    data: &Data,
    running: Arc<RwLock<RunningGame>>,
    message: &serenity::Message,
    kind: AnonymousMessageKind,
) -> Result<()> {
    let owner_id = match kind {
        AnonymousMessageKind::General { owner_id }
        | AnonymousMessageKind::Dead { owner_id }
        | AnonymousMessageKind::Shaman { owner_id }
        | AnonymousMessageKind::Role { owner_id, .. } => owner_id,
    };
    if message.author.id.get() != owner_id {
        let _ = message.delete(&ctx.http).await;
        return Ok(());
    }

    let body = anonymous_message_body(message);
    let can_relay = {
        let running_read = running.read().await;
        let Some(player) = running_read.game.get_player(owner_id) else {
            return Ok(());
        };
        match kind {
            AnonymousMessageKind::General { .. } => {
                if running_read.game.is_madam_seduced(player) {
                    false
                } else {
                    can_use_anonymous_general_chat(&running_read, player)
                }
            }
            AnonymousMessageKind::Dead { .. } => can_use_anonymous_dead_chat(&running_read, player),
            AnonymousMessageKind::Shaman { .. } => {
                can_use_anonymous_shaman_chat(&running_read, player)
            }
            AnonymousMessageKind::Role { role, .. } => {
                if running_read.game.is_madam_seduced(player) {
                    false
                } else {
                    can_use_anonymous_role_chat(&running_read, player, role)
                }
            }
        }
    };
    if !can_relay {
        return Ok(());
    }

    match kind {
        AnonymousMessageKind::General { .. } => {
            relay_anonymous_general_message(ctx, &running, owner_id, &body).await;
        }
        AnonymousMessageKind::Dead { .. } => {
            relay_anonymous_dead_message(ctx, data, &running, owner_id, &body).await;
        }
        AnonymousMessageKind::Shaman { .. } => {
            relay_anonymous_shaman_message(ctx, &running, owner_id, &body).await;
        }
        AnonymousMessageKind::Role { role, .. } => {
            relay_anonymous_role_message(ctx, &running, owner_id, role, &body).await;
            mirror_role_chat_to_dead(ctx, data, &running, message, role, &body).await;
        }
    }
    Ok(())
}

pub async fn handle_message_event(
    ctx: &serenity::Context,
    data: &Data,
    message: &serenity::Message,
) -> Result<()> {
    if message.author.bot {
        return Ok(());
    }
    let Some(guild_id) = message.guild_id else {
        return Ok(());
    };
    let Some(running) = data.games.get(&guild_id).map(|entry| entry.clone()) else {
        return Ok(());
    };
    let kind = {
        let running_read = running.read().await;
        if let Some(owner_id) = running_read
            .anonymous_dead_input_channel_owners
            .get(&message.channel_id)
            .copied()
        {
            Some(AnonymousMessageKind::Dead { owner_id })
        } else if let Some(owner_id) = running_read
            .anonymous_shaman_input_channel_owners
            .get(&message.channel_id)
            .copied()
        {
            Some(AnonymousMessageKind::Shaman { owner_id })
        } else if let Some(owner_id) = running_read
            .anonymous_input_channel_owners
            .get(&message.channel_id)
            .copied()
        {
            Some(AnonymousMessageKind::General { owner_id })
        } else {
            running_read
                .anonymous_role_input_channels
                .get(&message.channel_id)
                .copied()
                .map(|(owner_id, role)| AnonymousMessageKind::Role { owner_id, role })
        }
    };
    if let Some(kind) = kind {
        handle_anonymous_message(ctx, data, running, message, kind).await?;
        return Ok(());
    }

    let private_role = {
        let running_read = running.read().await;
        running_read
            .private_channel_ids
            .iter()
            .find_map(|(&role, &channel_id)| (channel_id == message.channel_id).then_some(role))
    };
    if let Some(role) = private_role {
        let player = {
            let running_read = running.read().await;
            running_read
                .game
                .get_player(message.author.id.get())
                .cloned()
        };
        if let Some(player) = player {
            if running.read().await.game.is_madam_seduced(&player) {
                let _ = message.delete(&ctx.http).await;
                set_private_role_member_access(ctx, &running, role, &player, false).await;
            } else {
                let body = anonymous_message_body(message);
                mirror_role_chat_to_dead(ctx, data, &running, message, role, &body).await;
            }
        }
        return Ok(());
    }

    let shaman_seduced = {
        let running_read = running.read().await;
        if running_read.shaman_channel_id == Some(message.channel_id) {
            running_read
                .game
                .get_player(message.author.id.get())
                .filter(|player| running_read.game.is_madam_seduced(player))
                .cloned()
        } else {
            None
        }
    };
    if let Some(player) = shaman_seduced {
        let _ = message.delete(&ctx.http).await;
        set_shaman_channel_member_access(ctx, &running, &player, true, false).await;
        return Ok(());
    }

    let frog_context = {
        let running_read = running.read().await;
        if running_read.frog_channel_id != Some(message.channel_id) {
            None
        } else {
            let player = running_read
                .game
                .get_player(message.author.id.get())
                .cloned();
            let can_croak = player.as_ref().is_some_and(|player| {
                running_read.game.is_frog(player)
                    && !running_read.game.is_madam_seduced(player)
                    && running_read.game.phase == Phase::Day
                    && running_read.day_chat_open
            });
            Some((running_read.channel_id, can_croak))
        }
    };
    if let Some((game_channel_id, can_croak)) = frog_context {
        let _ = message.delete(&ctx.http).await;
        if can_croak {
            let croak_count = message.content.chars().count().max(1);
            let _ = send_channel_embed(
                &ctx.http,
                game_channel_id,
                format!("개구리 {}", "개굴".repeat(croak_count)),
                "개구리 채팅",
                serenity::Colour::DARK_GREEN,
                vec![],
            )
            .await;
        }
    }
    Ok(())
}
