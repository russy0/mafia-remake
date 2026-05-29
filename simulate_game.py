from __future__ import annotations

import random

from game import MafiaGame, Phase, Role, Winner


def main() -> None:
    players = [(index, f"Player{index}") for index in range(1, 7)]
    game = MafiaGame(players, mafia_count=2, doctor_count=1, police_count=1, rng=random.Random(7))

    mafias = [player for player in game.alive_players() if player.role == Role.MAFIA]
    mafia = mafias[0]
    doctor = next(player for player in game.alive_players() if player.role == Role.DOCTOR)
    police = next(player for player in game.alive_players() if player.role == Role.POLICE)
    citizen_target = next(player for player in game.alive_players() if player.role == Role.CITIZEN)

    assert not game.all_night_actions_submitted()
    game.submit_night_action(mafia.user_id, citizen_target.user_id)
    assert not game.all_night_actions_submitted()
    game.submit_night_action(mafias[1].user_id, citizen_target.user_id)
    game.submit_night_action(doctor.user_id, citizen_target.user_id)
    police_result = game.submit_night_action(police.user_id, mafia.user_id)
    assert game.all_night_actions_submitted()
    try:
        game.submit_night_action(police.user_id, citizen_target.user_id)
    except ValueError as error:
        assert "이미 이번 밤 행동을 선택했습니다" in str(error)
    else:
        raise AssertionError("Police investigated twice in one night")
    night_result = game.resolve_night()

    assert "조사 투표 대상" in police_result
    assert night_result.police_target is not None
    assert night_result.police_target_is_mafia
    assert night_result.killed is None
    assert game.phase == Phase.DAY

    game.start_vote()
    alive = game.alive_players()
    for voter in alive[:3]:
        game.submit_day_vote(voter.user_id, mafia.user_id)
    assert not game.all_day_votes_submitted()
    vote_result = game.resolve_vote()

    assert vote_result.executed is not None
    assert vote_result.executed.user_id == mafia.user_id
    assert vote_result.vote_counts == {mafia.user_id: 3}
    assert game.phase == Phase.FINAL_DEFENSE
    game.start_confirmation_vote()
    for voter in game.alive_players()[:3]:
        game.submit_confirmation_vote(voter.user_id, True)
    confirm_result = game.resolve_confirmation_vote(mafia.user_id)
    assert confirm_result.executed is not None
    assert confirm_result.executed.user_id == mafia.user_id
    assert game.phase == Phase.NIGHT

    game.resolve_night()
    game.start_vote()
    assert not game.all_day_votes_submitted()
    skip_voter_count = len(game.alive_players())
    for voter in game.alive_players():
        game.submit_day_vote(voter.user_id, None)
    assert game.all_day_votes_submitted()
    skip_result = game.resolve_vote()

    assert skip_result.executed is None
    assert skip_result.skipped
    assert skip_result.vote_counts == {None: skip_voter_count}
    assert game.phase == Phase.NIGHT

    joker_game = MafiaGame(
        [(index, f"JokerGame{index}") for index in range(1, 7)],
        mafia_count=1,
        doctor_count=1,
        police_count=1,
        joker_count=1,
        rng=random.Random(11),
    )
    joker = next(player for player in joker_game.players if player.role == Role.JOKER)

    joker_game.resolve_night()
    joker_game.start_vote()
    for voter in joker_game.alive_players()[:3]:
        joker_game.submit_day_vote(voter.user_id, joker.user_id)
    joker_vote = joker_game.resolve_vote()

    assert joker_vote.executed is not None
    assert joker_vote.executed.user_id == joker.user_id
    joker_game.start_confirmation_vote()
    for voter in joker_game.alive_players()[:3]:
        joker_game.submit_confirmation_vote(voter.user_id, True)
    joker_confirm = joker_game.resolve_confirmation_vote(joker.user_id)
    assert joker_confirm.executed is not None
    assert joker_game.winner() == Winner.JOKER

    politician_game = MafiaGame(
        [(index, f"PoliticianGame{index}") for index in range(1, 6)],
        mafia_count=1,
        doctor_count=0,
        police_count=0,
        special_roles=[Role.POLITICIAN],
        rng=random.Random(41),
    )
    politician = next(player for player in politician_game.players if player.role == Role.POLITICIAN)
    politician_game.resolve_night()
    politician_game.start_vote()
    for voter in politician_game.alive_players()[:3]:
        politician_game.submit_day_vote(voter.user_id, politician.user_id)
    politician_vote = politician_game.resolve_vote()
    assert politician_vote.executed is not None
    assert politician_vote.executed.user_id == politician.user_id
    assert politician_game.phase == Phase.FINAL_DEFENSE
    politician_game.start_confirmation_vote()
    for voter in politician_game.alive_players():
        politician_game.submit_confirmation_vote(voter.user_id, True)
    politician_confirm = politician_game.resolve_confirmation_vote(politician.user_id)
    assert politician_confirm.approved
    assert politician_confirm.blocked_by_politician
    assert politician_confirm.executed is None
    assert politician.alive
    assert politician_game.phase == Phase.NIGHT

    politician_weight_game = MafiaGame(
        [(index, f"PoliticianWeightGame{index}") for index in range(1, 6)],
        mafia_count=1,
        doctor_count=0,
        police_count=0,
        special_roles=[Role.POLITICIAN],
        rng=random.Random(43),
    )
    weight_mafia = next(player for player in politician_weight_game.players if player.role == Role.MAFIA)
    weight_politician = next(player for player in politician_weight_game.players if player.role == Role.POLITICIAN)
    weight_other = next(
        player
        for player in politician_weight_game.players
        if player.user_id not in {weight_mafia.user_id, weight_politician.user_id}
    )
    politician_weight_game.resolve_night()
    politician_weight_game.start_vote()
    politician_weight_game.submit_day_vote(weight_politician.user_id, weight_mafia.user_id)
    politician_weight_game.submit_day_vote(weight_other.user_id, weight_politician.user_id)
    politician_weight_vote = politician_weight_game.resolve_vote()
    assert politician_weight_vote.executed is not None
    assert politician_weight_vote.executed.user_id == weight_mafia.user_id
    assert politician_weight_vote.vote_counts == {
        weight_mafia.user_id: 2,
        weight_politician.user_id: 1,
    }
    politician_weight_game.start_confirmation_vote()
    politician_weight_game.submit_confirmation_vote(weight_politician.user_id, True)
    politician_weight_game.submit_confirmation_vote(weight_other.user_id, False)
    politician_weight_confirm = politician_weight_game.resolve_confirmation_vote(weight_mafia.user_id)
    assert politician_weight_confirm.executed is not None
    assert politician_weight_confirm.vote_counts == {True: 2, False: 1}

    spy_game = MafiaGame(
        [(index, f"SpyGame{index}") for index in range(1, 6)],
        mafia_count=1,
        doctor_count=0,
        police_count=0,
        special_roles=[Role.SPY],
        rng=random.Random(45),
    )
    spy = next(player for player in spy_game.players if player.role == Role.SPY)
    spy_mafia = next(player for player in spy_game.players if player.role == Role.MAFIA)
    spy_citizen = next(player for player in spy_game.players if player.role == Role.CITIZEN)
    spy_result = spy_game.submit_night_action(spy.user_id, spy_mafia.user_id)
    assert "[첩보]" in spy_result
    assert "[접선]" in spy_result
    assert spy.user_id in spy_game.spy_contacted
    assert spy_game.spy_can_use_bonus_action(spy.user_id)
    assert not spy_game.all_night_actions_submitted()
    spy_bonus_result = spy_game.submit_night_action(spy.user_id, spy_citizen.user_id)
    assert spy_citizen.role.value in spy_bonus_result
    assert not spy_game.spy_can_use_bonus_action(spy.user_id)
    spy_game.submit_night_action(spy_mafia.user_id, spy_citizen.user_id)
    assert spy_game.all_night_actions_submitted()
    spy_night = spy_game.resolve_night()
    assert spy_night.spy_contacts == [spy.user_id]

    hidden_spy_game = MafiaGame(
        [(index, f"HiddenSpyGame{index}") for index in range(1, 6)],
        mafia_count=1,
        doctor_count=0,
        police_count=0,
        special_roles=[Role.SPY],
        rng=random.Random(46),
    )
    hidden_spy_mafia = next(player for player in hidden_spy_game.players if player.role == Role.MAFIA)
    hidden_spy = next(player for player in hidden_spy_game.players if player.role == Role.SPY)
    hidden_spy_game.submit_night_action(hidden_spy_mafia.user_id, hidden_spy.user_id)
    hidden_spy_result = hidden_spy_game.resolve_night()
    assert hidden_spy_result.killed is not None
    assert hidden_spy_result.killed.user_id == hidden_spy.user_id

    contacted_spy_game = MafiaGame(
        [(index, f"ContactedSpyGame{index}") for index in range(1, 6)],
        mafia_count=1,
        doctor_count=0,
        police_count=0,
        special_roles=[Role.SPY],
        rng=random.Random(48),
    )
    contacted_mafia = next(player for player in contacted_spy_game.players if player.role == Role.MAFIA)
    contacted_spy = next(player for player in contacted_spy_game.players if player.role == Role.SPY)
    contacted_spy_game.spy_contacted.add(contacted_spy.user_id)
    try:
        contacted_spy_game.submit_night_action(contacted_mafia.user_id, contacted_spy.user_id)
    except ValueError as error:
        assert "접선된 마피아 팀" in str(error)
    else:
        raise AssertionError("Mafia attacked a contacted spy")

    hidden_godfather_game = MafiaGame(
        [(index, f"HiddenGodfatherGame{index}") for index in range(1, 6)],
        mafia_count=1,
        doctor_count=0,
        police_count=0,
        special_roles=[Role.GODFATHER],
        rng=random.Random(49),
    )
    hidden_godfather_mafia = next(
        player for player in hidden_godfather_game.players if player.role == Role.MAFIA
    )
    hidden_godfather = next(
        player for player in hidden_godfather_game.players if player.role == Role.GODFATHER
    )
    hidden_godfather_game.submit_night_action(hidden_godfather_mafia.user_id, hidden_godfather.user_id)
    hidden_godfather_result = hidden_godfather_game.resolve_night()
    assert hidden_godfather_result.killed is not None
    assert hidden_godfather_result.killed.user_id == hidden_godfather.user_id

    police_hidden_spy_game = MafiaGame(
        [(index, f"PoliceHiddenSpyGame{index}") for index in range(1, 6)],
        mafia_count=1,
        doctor_count=0,
        police_count=1,
        special_roles=[Role.SPY],
        rng=random.Random(52),
    )
    police_hidden_spy_police = next(
        player for player in police_hidden_spy_game.players if player.role == Role.POLICE
    )
    police_hidden_spy = next(player for player in police_hidden_spy_game.players if player.role == Role.SPY)
    police_hidden_spy_game.submit_night_action(
        police_hidden_spy_police.user_id,
        police_hidden_spy.user_id,
    )
    police_hidden_spy_result = police_hidden_spy_game.resolve_night()
    assert police_hidden_spy_result.police_target_is_mafia is False

    police_contacted_spy_game = MafiaGame(
        [(index, f"PoliceContactedSpyGame{index}") for index in range(1, 6)],
        mafia_count=1,
        doctor_count=0,
        police_count=1,
        special_roles=[Role.SPY],
        rng=random.Random(53),
    )
    police_contacted_spy_police = next(
        player for player in police_contacted_spy_game.players if player.role == Role.POLICE
    )
    police_contacted_spy = next(
        player for player in police_contacted_spy_game.players if player.role == Role.SPY
    )
    police_contacted_spy_game.spy_contacted.add(police_contacted_spy.user_id)
    police_contacted_spy_game.submit_night_action(
        police_contacted_spy_police.user_id,
        police_contacted_spy.user_id,
    )
    police_contacted_spy_result = police_contacted_spy_game.resolve_night()
    assert police_contacted_spy_result.police_target_is_mafia is True

    police_hidden_godfather_game = MafiaGame(
        [(index, f"PoliceHiddenGodfatherGame{index}") for index in range(1, 6)],
        mafia_count=1,
        doctor_count=0,
        police_count=1,
        special_roles=[Role.GODFATHER],
        rng=random.Random(54),
    )
    police_hidden_godfather_police = next(
        player for player in police_hidden_godfather_game.players if player.role == Role.POLICE
    )
    police_hidden_godfather = next(
        player for player in police_hidden_godfather_game.players if player.role == Role.GODFATHER
    )
    police_hidden_godfather_game.submit_night_action(
        police_hidden_godfather_police.user_id,
        police_hidden_godfather.user_id,
    )
    police_hidden_godfather_result = police_hidden_godfather_game.resolve_night()
    assert police_hidden_godfather_result.police_target_is_mafia is False

    police_contacted_godfather_game = MafiaGame(
        [(index, f"PoliceContactedGodfatherGame{index}") for index in range(1, 6)],
        mafia_count=1,
        doctor_count=0,
        police_count=1,
        special_roles=[Role.GODFATHER],
        rng=random.Random(55),
    )
    police_contacted_godfather_police = next(
        player for player in police_contacted_godfather_game.players if player.role == Role.POLICE
    )
    police_contacted_godfather = next(
        player for player in police_contacted_godfather_game.players if player.role == Role.GODFATHER
    )
    police_contacted_godfather_game.godfather_contacted.add(police_contacted_godfather.user_id)
    police_contacted_godfather_game.submit_night_action(
        police_contacted_godfather_police.user_id,
        police_contacted_godfather.user_id,
    )
    police_contacted_godfather_result = police_contacted_godfather_game.resolve_night()
    assert police_contacted_godfather_result.police_target_is_mafia is True

    uncontacted_spy_end_game = MafiaGame(
        [(index, f"UncontactedSpyEndGame{index}") for index in range(1, 6)],
        mafia_count=1,
        doctor_count=0,
        police_count=0,
        special_roles=[Role.SPY],
        rng=random.Random(56),
    )
    uncontacted_spy_mafia = next(
        player for player in uncontacted_spy_end_game.players if player.role == Role.MAFIA
    )
    uncontacted_spy = next(player for player in uncontacted_spy_end_game.players if player.role == Role.SPY)
    uncontacted_spy_mafia.alive = False
    assert uncontacted_spy.alive
    assert len(uncontacted_spy_end_game.alive_mafia_team()) == 1
    assert len(uncontacted_spy_end_game.alive_known_mafia_team()) == 0
    assert uncontacted_spy_end_game.winner() == Winner.CITIZEN

    contacted_spy_count_game = MafiaGame(
        [(index, f"ContactedSpyCountGame{index}") for index in range(1, 6)],
        mafia_count=1,
        doctor_count=0,
        police_count=0,
        special_roles=[Role.SPY],
        rng=random.Random(57),
    )
    contacted_count_mafia = next(
        player for player in contacted_spy_count_game.players if player.role == Role.MAFIA
    )
    contacted_count_spy = next(
        player for player in contacted_spy_count_game.players if player.role == Role.SPY
    )
    contacted_spy_count_game.spy_contacted.add(contacted_count_spy.user_id)
    contacted_count_mafia.alive = False
    assert len(contacted_spy_count_game.alive_known_mafia_team()) == 1
    assert contacted_spy_count_game.winner() is None

    uncontacted_godfather_end_game = MafiaGame(
        [(index, f"UncontactedGodfatherEndGame{index}") for index in range(1, 6)],
        mafia_count=1,
        doctor_count=0,
        police_count=0,
        special_roles=[Role.GODFATHER],
        rng=random.Random(58),
    )
    uncontacted_godfather_mafia = next(
        player for player in uncontacted_godfather_end_game.players if player.role == Role.MAFIA
    )
    uncontacted_godfather = next(
        player for player in uncontacted_godfather_end_game.players if player.role == Role.GODFATHER
    )
    uncontacted_godfather_mafia.alive = False
    assert uncontacted_godfather.alive
    assert len(uncontacted_godfather_end_game.alive_mafia_team()) == 1
    assert len(uncontacted_godfather_end_game.alive_known_mafia_team()) == 0
    assert uncontacted_godfather_end_game.winner() == Winner.CITIZEN

    contacted_godfather_count_game = MafiaGame(
        [(index, f"ContactedGodfatherCountGame{index}") for index in range(1, 6)],
        mafia_count=1,
        doctor_count=0,
        police_count=0,
        special_roles=[Role.GODFATHER],
        rng=random.Random(59),
    )
    contacted_count_godfather_mafia = next(
        player for player in contacted_godfather_count_game.players if player.role == Role.MAFIA
    )
    contacted_count_godfather = next(
        player for player in contacted_godfather_count_game.players if player.role == Role.GODFATHER
    )
    contacted_godfather_count_game.godfather_contacted.add(contacted_count_godfather.user_id)
    contacted_count_godfather_mafia.alive = False
    assert len(contacted_godfather_count_game.alive_known_mafia_team()) == 1
    assert contacted_godfather_count_game.winner() is None

    contractor_contact_game = MafiaGame(
        [(index, f"ContractorContactGame{index}") for index in range(1, 6)],
        mafia_count=1,
        doctor_count=0,
        police_count=0,
        special_roles=[Role.CONTRACTOR],
        rng=random.Random(67),
    )
    contractor = next(player for player in contractor_contact_game.players if player.role == Role.CONTRACTOR)
    contractor_mafia = next(player for player in contractor_contact_game.players if player.role == Role.MAFIA)
    contractor_contact_result = contractor_contact_game.submit_contractor_contact(
        contractor.user_id,
        contractor_mafia.user_id,
    )
    assert "[동업]" in contractor_contact_result
    assert contractor.user_id in contractor_contact_game.contractor_contacted
    contractor_contact_night = contractor_contact_game.resolve_night()
    assert contractor_contact_night.contractor_contacts == [contractor.user_id]

    contractor_police_game = MafiaGame(
        [(index, f"ContractorPoliceGame{index}") for index in range(1, 6)],
        mafia_count=1,
        doctor_count=0,
        police_count=1,
        special_roles=[Role.CONTRACTOR],
        rng=random.Random(68),
    )
    contractor_police = next(player for player in contractor_police_game.players if player.role == Role.POLICE)
    hidden_contractor = next(
        player for player in contractor_police_game.players if player.role == Role.CONTRACTOR
    )
    contractor_police_game.submit_night_action(
        contractor_police.user_id,
        hidden_contractor.user_id,
    )
    contractor_police_result = contractor_police_game.resolve_night()
    assert contractor_police_result.police_target_is_mafia is False

    contractor_success_game = MafiaGame(
        [(index, f"ContractorSuccessGame{index}") for index in range(1, 8)],
        mafia_count=1,
        doctor_count=1,
        police_count=1,
        special_roles=[Role.CONTRACTOR],
        rng=random.Random(69),
    )
    success_contractor = next(
        player for player in contractor_success_game.players if player.role == Role.CONTRACTOR
    )
    success_doctor = next(player for player in contractor_success_game.players if player.role == Role.DOCTOR)
    success_police = next(player for player in contractor_success_game.players if player.role == Role.POLICE)
    contractor_success_game.day_number = 2
    contractor_success_game.submit_contractor_contract(
        success_contractor.user_id,
        success_doctor.user_id,
        Role.DOCTOR,
        success_police.user_id,
        Role.POLICE,
    )
    contractor_success_result = contractor_success_game.resolve_night()
    assert {player.user_id for player in contractor_success_result.contractor_kills} == {
        success_doctor.user_id,
        success_police.user_id,
    }
    assert not success_doctor.alive
    assert not success_police.alive

    contractor_fail_game = MafiaGame(
        [(index, f"ContractorFailGame{index}") for index in range(1, 8)],
        mafia_count=1,
        doctor_count=1,
        police_count=1,
        special_roles=[Role.CONTRACTOR],
        rng=random.Random(70),
    )
    fail_contractor = next(
        player for player in contractor_fail_game.players if player.role == Role.CONTRACTOR
    )
    fail_doctor = next(player for player in contractor_fail_game.players if player.role == Role.DOCTOR)
    fail_police = next(player for player in contractor_fail_game.players if player.role == Role.POLICE)
    contractor_fail_game.day_number = 2
    contractor_fail_game.submit_contractor_contract(
        fail_contractor.user_id,
        fail_doctor.user_id,
        Role.DOCTOR,
        fail_police.user_id,
        Role.DOCTOR,
    )
    contractor_fail_result = contractor_fail_game.resolve_night()
    assert contractor_fail_result.contractor_kills == []
    assert "암살에 실패" in contractor_fail_result.contractor_results[fail_contractor.user_id]
    assert fail_doctor.alive
    assert fail_police.alive

    contractor_revealed_game = MafiaGame(
        [(index, f"ContractorRevealedGame{index}") for index in range(1, 8)],
        mafia_count=1,
        doctor_count=1,
        police_count=1,
        special_roles=[Role.CONTRACTOR],
        rng=random.Random(71),
    )
    revealed_contractor = next(
        player for player in contractor_revealed_game.players if player.role == Role.CONTRACTOR
    )
    revealed_doctor = next(player for player in contractor_revealed_game.players if player.role == Role.DOCTOR)
    revealed_police = next(player for player in contractor_revealed_game.players if player.role == Role.POLICE)
    contractor_revealed_game.day_number = 2
    contractor_revealed_game.publicly_revealed_ids.add(revealed_doctor.user_id)
    try:
        contractor_revealed_game.submit_contractor_contract(
            revealed_contractor.user_id,
            revealed_doctor.user_id,
            Role.DOCTOR,
            revealed_police.user_id,
            Role.POLICE,
        )
    except ValueError as error:
        assert "공개적으로 드러난" in str(error)
    else:
        raise AssertionError("Contractor targeted a publicly revealed player")

    soldier_game = MafiaGame(
        [(index, f"SoldierGame{index}") for index in range(1, 6)],
        mafia_count=1,
        doctor_count=0,
        police_count=0,
        special_roles=[Role.SOLDIER],
        rng=random.Random(60),
    )
    soldier_mafia = next(player for player in soldier_game.players if player.role == Role.MAFIA)
    soldier = next(player for player in soldier_game.players if player.role == Role.SOLDIER)
    soldier_game.submit_night_action(soldier_mafia.user_id, soldier.user_id)
    soldier_first_result = soldier_game.resolve_night()
    assert soldier.alive
    assert soldier_first_result.killed is None
    assert soldier_first_result.soldier_blocks == [soldier]
    assert soldier.user_id in soldier_game.soldier_bulletproof_used

    soldier_game.start_vote()
    for voter in soldier_game.alive_players():
        soldier_game.submit_day_vote(voter.user_id, None)
    soldier_game.resolve_vote()
    soldier_game.submit_night_action(soldier_mafia.user_id, soldier.user_id)
    soldier_second_result = soldier_game.resolve_night()
    assert not soldier.alive
    assert soldier_second_result.killed is not None
    assert soldier_second_result.killed.user_id == soldier.user_id
    assert soldier_second_result.soldier_blocks == []

    protected_soldier_game = MafiaGame(
        [(index, f"ProtectedSoldierGame{index}") for index in range(1, 6)],
        mafia_count=1,
        doctor_count=1,
        police_count=0,
        special_roles=[Role.SOLDIER],
        rng=random.Random(62),
    )
    protected_soldier_mafia = next(
        player for player in protected_soldier_game.players if player.role == Role.MAFIA
    )
    protected_soldier_doctor = next(
        player for player in protected_soldier_game.players if player.role == Role.DOCTOR
    )
    protected_soldier = next(
        player for player in protected_soldier_game.players if player.role == Role.SOLDIER
    )
    protected_soldier_game.submit_night_action(
        protected_soldier_mafia.user_id,
        protected_soldier.user_id,
    )
    protected_soldier_game.submit_night_action(
        protected_soldier_doctor.user_id,
        protected_soldier.user_id,
    )
    protected_soldier_result = protected_soldier_game.resolve_night()
    assert protected_soldier.alive
    assert protected_soldier_result.killed is None
    assert protected_soldier_result.soldier_blocks == []
    assert protected_soldier.user_id not in protected_soldier_game.soldier_bulletproof_used

    shaman_game = MafiaGame(
        [(index, f"ShamanGame{index}") for index in range(1, 6)],
        mafia_count=1,
        doctor_count=0,
        police_count=0,
        special_roles=[Role.SHAMAN],
        rng=random.Random(63),
    )
    shaman_mafia = next(player for player in shaman_game.players if player.role == Role.MAFIA)
    shaman = next(player for player in shaman_game.players if player.role == Role.SHAMAN)
    shaman_victim = next(player for player in shaman_game.players if player.role == Role.CITIZEN)
    assert shaman not in shaman_game.night_action_actors()
    shaman_game.submit_night_action(shaman_mafia.user_id, shaman_victim.user_id)
    shaman_first_night = shaman_game.resolve_night()
    assert shaman_first_night.killed is not None
    assert shaman_first_night.killed.user_id == shaman_victim.user_id

    shaman_game.start_vote()
    for voter in shaman_game.alive_players():
        shaman_game.submit_day_vote(voter.user_id, None)
    shaman_game.resolve_vote()
    assert shaman in shaman_game.night_action_actors()
    shaman_result = shaman_game.submit_night_action(shaman.user_id, shaman_victim.user_id)
    assert "성불 대상" in shaman_result
    shaman_second_night = shaman_game.resolve_night()
    assert shaman_second_night.shaman_purifications == [shaman_victim.user_id]
    assert shaman_victim.user_id in shaman_game.purified_dead_ids
    assert shaman.user_id in shaman_second_night.shaman_results
    assert shaman_victim.role.value in shaman_second_night.shaman_results[shaman.user_id]

    graverobber_game = MafiaGame(
        [(index, f"GraverobberGame{index}") for index in range(1, 6)],
        mafia_count=1,
        doctor_count=1,
        police_count=0,
        special_roles=[Role.GRAVEROBBER],
        rng=random.Random(47),
    )
    graverobber_mafia = next(player for player in graverobber_game.players if player.role == Role.MAFIA)
    graverobber = next(player for player in graverobber_game.players if player.role == Role.GRAVEROBBER)
    robbed_doctor = next(player for player in graverobber_game.players if player.role == Role.DOCTOR)
    graverobber_game.submit_night_action(graverobber_mafia.user_id, robbed_doctor.user_id)
    graverobber_result = graverobber_game.resolve_night()
    assert graverobber.role == Role.DOCTOR
    assert robbed_doctor.role == Role.CITIZEN
    assert graverobber_result.graverobber_results == {graverobber.user_id: Role.DOCTOR}

    terrorist_night_game = MafiaGame(
        [(index, f"TerroristNightGame{index}") for index in range(1, 6)],
        mafia_count=1,
        doctor_count=0,
        police_count=0,
        special_roles=[Role.TERRORIST],
        rng=random.Random(51),
    )
    night_mafia = next(player for player in terrorist_night_game.players if player.role == Role.MAFIA)
    night_terrorist = next(player for player in terrorist_night_game.players if player.role == Role.TERRORIST)
    terrorist_night_game.submit_night_action(night_terrorist.user_id, night_mafia.user_id)
    terrorist_night_game.submit_night_action(night_mafia.user_id, night_terrorist.user_id)
    terrorist_night_result = terrorist_night_game.resolve_night()
    assert not night_terrorist.alive
    assert not night_mafia.alive
    assert {player.user_id for player in terrorist_night_result.killed_players} == {
        night_terrorist.user_id,
        night_mafia.user_id,
    }
    assert terrorist_night_result.terrorist_retaliations == [(night_terrorist, night_mafia)]

    terrorist_vote_game = MafiaGame(
        [(index, f"TerroristVoteGame{index}") for index in range(1, 6)],
        mafia_count=1,
        doctor_count=0,
        police_count=0,
        special_roles=[Role.TERRORIST],
        rng=random.Random(61),
    )
    vote_mafia = next(player for player in terrorist_vote_game.players if player.role == Role.MAFIA)
    vote_terrorist = next(player for player in terrorist_vote_game.players if player.role == Role.TERRORIST)
    terrorist_vote_game.submit_night_action(vote_terrorist.user_id, vote_mafia.user_id)
    terrorist_vote_game.resolve_night()
    terrorist_vote_game.start_vote()
    for voter in terrorist_vote_game.alive_players()[:3]:
        terrorist_vote_game.submit_day_vote(voter.user_id, vote_terrorist.user_id)
    terrorist_vote = terrorist_vote_game.resolve_vote()
    assert terrorist_vote.executed is not None
    assert terrorist_vote.executed.user_id == vote_terrorist.user_id
    terrorist_vote_game.start_confirmation_vote()
    for voter in terrorist_vote_game.alive_players():
        terrorist_vote_game.submit_confirmation_vote(voter.user_id, True)
    terrorist_confirm = terrorist_vote_game.resolve_confirmation_vote(vote_terrorist.user_id)
    assert terrorist_confirm.executed is not None
    assert terrorist_confirm.executed.user_id == vote_terrorist.user_id
    assert terrorist_confirm.extra_killed == [vote_mafia]
    assert not vote_terrorist.alive
    assert not vote_mafia.alive

    godfather_game = MafiaGame(
        [(index, f"GodfatherGame{index}") for index in range(1, 7)],
        mafia_count=1,
        doctor_count=1,
        police_count=0,
        special_roles=[Role.GODFATHER],
        rng=random.Random(31),
    )
    godfather = next(player for player in godfather_game.players if player.role == Role.GODFATHER)
    assert godfather.user_id not in godfather_game.godfather_contacted
    godfather_game.resolve_night()
    godfather_game.start_vote()
    godfather_game.resolve_vote()
    assert godfather_game.phase == Phase.NIGHT
    assert godfather_game.day_number == 2
    godfather_game.resolve_night()
    godfather_game.start_vote()
    godfather_game.resolve_vote()
    assert godfather_game.phase == Phase.NIGHT
    assert godfather_game.day_number == 3
    contacted = godfather_game.ensure_godfather_auto_contact()
    assert godfather.user_id in contacted

    split_doctor_game = MafiaGame(
        [(index, f"DoctorGame{index}") for index in range(1, 6)],
        mafia_count=1,
        doctor_count=2,
        police_count=0,
        joker_count=0,
        rng=random.Random(21),
    )
    split_mafia = next(player for player in split_doctor_game.players if player.role == Role.MAFIA)
    split_doctors = [player for player in split_doctor_game.players if player.role == Role.DOCTOR]
    split_victim = next(player for player in split_doctor_game.players if player.role == Role.CITIZEN)
    split_other = next(
        player
        for player in split_doctor_game.players
        if player.user_id not in {split_mafia.user_id, split_victim.user_id, split_doctors[0].user_id}
    )

    split_doctor_game.submit_night_action(split_mafia.user_id, split_victim.user_id)
    split_doctor_game.submit_night_action(split_doctors[0].user_id, split_victim.user_id)
    split_doctor_game.submit_night_action(split_doctors[1].user_id, split_other.user_id)
    split_result = split_doctor_game.resolve_night()

    assert split_result.protected is None
    assert split_result.killed is not None
    assert split_result.killed.user_id == split_victim.user_id
    print("simulation ok")


if __name__ == "__main__":
    main()
