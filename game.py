from __future__ import annotations

from collections import Counter
from dataclasses import dataclass, field
from enum import Enum
import random


class Role(str, Enum):
    MAFIA = "마피아"
    DOCTOR = "의사"
    POLICE = "경찰"
    DETECTIVE = "사립탐정"
    SHAMAN = "영매"
    SOLDIER = "군인"
    SPY = "스파이"
    CONTRACTOR = "청부업자"
    GRAVEROBBER = "도굴꾼"
    GODFATHER = "대부"
    JOKER = "조커"
    POLITICIAN = "정치인"
    TERRORIST = "테러리스트"
    VILLAIN = "악인"
    CITIZEN = "시민"


class Phase(str, Enum):
    NIGHT = "밤"
    DAY = "낮"
    VOTE = "투표"
    FINAL_DEFENSE = "최후변론"
    CONFIRM_VOTE = "찬반투표"
    ENDED = "종료"


class Winner(str, Enum):
    MAFIA = "마피아"
    CITIZEN = "시민"
    JOKER = "조커"


MAFIA_TEAM_ROLES = {Role.MAFIA, Role.SPY, Role.CONTRACTOR, Role.GODFATHER, Role.VILLAIN}
CONTRACTOR_GUESSABLE_ROLES = {
    Role.DOCTOR,
    Role.POLICE,
    Role.DETECTIVE,
    Role.SHAMAN,
    Role.GRAVEROBBER,
    Role.POLITICIAN,
    Role.TERRORIST,
    Role.SOLDIER,
    Role.JOKER,
    Role.CITIZEN,
}


@dataclass
class Player:
    user_id: int
    name: str
    role: Role
    alive: bool = True


@dataclass
class NightResult:
    killed: Player | None
    protected: Player | None
    mafia_target: Player | None
    police_target: Player | None
    police_target_is_mafia: bool | None
    killed_players: list[Player] = field(default_factory=list)
    detective_results: dict[int, str] = field(default_factory=dict)
    spy_results: dict[int, str] = field(default_factory=dict)
    spy_contacts: list[int] = field(default_factory=list)
    contractor_results: dict[int, str] = field(default_factory=dict)
    contractor_contacts: list[int] = field(default_factory=list)
    contractor_kills: list[Player] = field(default_factory=list)
    godfather_results: dict[int, str] = field(default_factory=dict)
    godfather_contacts: list[int] = field(default_factory=list)
    graverobber_results: dict[int, Role] = field(default_factory=dict)
    terrorist_retaliations: list[tuple[Player, Player]] = field(default_factory=list)
    soldier_blocks: list[Player] = field(default_factory=list)
    shaman_results: dict[int, str] = field(default_factory=dict)
    shaman_purifications: list[int] = field(default_factory=list)


@dataclass
class VoteResult:
    executed: Player | None
    tied: bool
    skipped: bool = False
    vote_counts: dict[int | None, int] = field(default_factory=dict)


@dataclass
class ConfirmVoteResult:
    executed: Player | None
    approved: bool
    tied: bool
    blocked_by_politician: bool = False
    extra_killed: list[Player] = field(default_factory=list)
    vote_counts: dict[bool, int] = field(default_factory=dict)


class MafiaGame:
    def __init__(
        self,
        players: list[tuple[int, str]],
        mafia_count: int,
        doctor_count: int,
        police_count: int,
        rng: random.Random | None = None,
        joker_count: int = 0,
        special_roles: list[Role] | None = None,
    ) -> None:
        self._rng = rng or random.Random()
        special_roles = special_roles or []
        self._validate_counts(
            players,
            mafia_count,
            doctor_count,
            police_count,
            joker_count,
            special_roles,
        )

        roles = (
            [Role.MAFIA] * mafia_count
            + [Role.DOCTOR] * doctor_count
            + [Role.POLICE] * police_count
            + [Role.JOKER] * joker_count
            + special_roles
        )
        roles.extend([Role.CITIZEN] * (len(players) - len(roles)))
        self._rng.shuffle(roles)

        shuffled_players = players[:]
        self._rng.shuffle(shuffled_players)
        self.players = [
            Player(user_id=user_id, name=name, role=role)
            for (user_id, name), role in zip(shuffled_players, roles, strict=True)
        ]
        self.phase = Phase.NIGHT
        self.day_number = 1
        self.mafia_targets: dict[int, int] = {}
        self.doctor_targets: dict[int, int] = {}
        self.police_targets: dict[int, int] = {}
        self.detective_targets: dict[int, int] = {}
        self.shaman_targets: dict[int, int] = {}
        self.spy_targets: dict[int, list[int]] = {}
        self.spy_bonus_pending: set[int] = set()
        self.spy_contacts_this_night: list[int] = []
        self.contractor_contact_targets: dict[int, int] = {}
        self.contractor_contracts: dict[int, tuple[tuple[int, Role], tuple[int, Role]]] = {}
        self.contractor_contacts_this_night: list[int] = []
        self.godfather_targets: dict[int, int] = {}
        self.terrorist_targets: dict[int, int] = {}
        self.terrorist_action_submitted: set[int] = set()
        self.soldier_bulletproof_used: set[int] = set()
        self.purified_dead_ids: set[int] = set()
        self.publicly_revealed_ids: set[int] = set()
        self.day_votes: dict[int, int | None] = {}
        self.confirm_votes: dict[int, bool] = {}
        self.spy_contacted: set[int] = set()
        self.contractor_contacted: set[int] = set()
        self.godfather_contacted: set[int] = set()
        self.joker_won = False

    @staticmethod
    def _validate_counts(
        players: list[tuple[int, str]],
        mafia_count: int,
        doctor_count: int,
        police_count: int,
        joker_count: int,
        special_roles: list[Role],
    ) -> None:
        if len(players) < 3:
            raise ValueError("최소 3명이 필요합니다.")
        if len(players) > 24:
            raise ValueError("투표 스킵 선택지를 포함해야 해서 최대 24명까지 지원합니다.")
        if len({user_id for user_id, _ in players}) != len(players):
            raise ValueError("중복된 참가자가 있습니다.")
        if mafia_count < 0:
            raise ValueError("기본 마피아 수는 0명 이상이어야 합니다.")
        if doctor_count < 0 or police_count < 0 or joker_count < 0:
            raise ValueError("의사, 경찰, 조커 수는 0명 이상이어야 합니다.")
        if len(set(special_roles)) != len(special_roles):
            raise ValueError("같은 특수 역할은 한 게임에 한 번만 선택됩니다.")

        special_count = mafia_count + doctor_count + police_count + joker_count + len(special_roles)
        if special_count > len(players):
            raise ValueError("직업 수의 합계가 참가자 수보다 많습니다.")

        mafia_team_count = mafia_count + sum(role in MAFIA_TEAM_ROLES for role in special_roles)
        if mafia_team_count < 1:
            raise ValueError("마피아 계열은 최소 1명이어야 합니다.")
        if mafia_team_count >= len(players) - mafia_team_count:
            raise ValueError("시작할 때 시민 진영이 마피아 팀보다 많아야 합니다.")

    def alive_players(self) -> list[Player]:
        return [player for player in self.players if player.alive]

    def dead_players(self) -> list[Player]:
        return [player for player in self.players if not player.alive]

    def unpurified_dead_players(self) -> list[Player]:
        return [player for player in self.dead_players() if player.user_id not in self.purified_dead_ids]

    def alive_mafia(self) -> list[Player]:
        return [player for player in self.alive_players() if player.role == Role.MAFIA]

    def alive_mafia_team(self) -> list[Player]:
        return [player for player in self.alive_players() if self.is_mafia_team(player)]

    def alive_known_mafia_team(self) -> list[Player]:
        return [player for player in self.alive_players() if self.is_known_mafia_team(player)]

    def alive_role_count(self, role: Role) -> int:
        return sum(1 for player in self.alive_players() if player.role == role)

    def get_player(self, user_id: int) -> Player | None:
        return next((player for player in self.players if player.user_id == user_id), None)

    def is_mafia_team(self, player: Player) -> bool:
        return player.role in MAFIA_TEAM_ROLES

    def is_known_mafia_team(self, player: Player) -> bool:
        if player.role in {Role.MAFIA, Role.VILLAIN}:
            return True
        if player.role == Role.SPY:
            return player.user_id in self.spy_contacted
        if player.role == Role.CONTRACTOR:
            return player.user_id in self.contractor_contacted
        if player.role == Role.GODFATHER:
            return player.user_id in self.godfather_contacted
        return False

    def can_mafia_attack(self, player: Player) -> bool:
        return not self.is_known_mafia_team(player)

    def is_citizen_team(self, player: Player) -> bool:
        return not self.is_mafia_team(player) and player.role != Role.JOKER

    def night_action_actors(self) -> list[Player]:
        self.ensure_godfather_auto_contact()
        alive = self.alive_players()
        actors: list[Player] = []
        for player in alive:
            if player.role == Role.MAFIA and any(self.can_mafia_attack(target) for target in alive):
                actors.append(player)
            elif player.role == Role.DOCTOR:
                actors.append(player)
            elif player.role in {Role.POLICE, Role.DETECTIVE, Role.SPY, Role.TERRORIST} and any(
                target.user_id != player.user_id for target in alive
            ):
                actors.append(player)
            elif player.role == Role.CONTRACTOR and self._contractor_can_act(player):
                actors.append(player)
            elif player.role == Role.SHAMAN and self.unpurified_dead_players():
                actors.append(player)
            elif (
                player.role == Role.GODFATHER
                and player.user_id in self.godfather_contacted
                and any(target.user_id != player.user_id for target in alive)
            ):
                actors.append(player)
        return actors

    def all_night_actions_submitted(self) -> bool:
        if self.phase != Phase.NIGHT:
            return False

        for actor in self.night_action_actors():
            if actor.role == Role.MAFIA and actor.user_id not in self.mafia_targets:
                return False
            if actor.role == Role.DOCTOR and actor.user_id not in self.doctor_targets:
                return False
            if actor.role == Role.POLICE and actor.user_id not in self.police_targets:
                return False
            if actor.role == Role.DETECTIVE and actor.user_id not in self.detective_targets:
                return False
            if actor.role == Role.SHAMAN and actor.user_id not in self.shaman_targets:
                return False
            if actor.role == Role.SPY and not self.spy_targets.get(actor.user_id):
                return False
            if actor.role == Role.SPY and actor.user_id in self.spy_bonus_pending:
                return False
            if (
                actor.role == Role.CONTRACTOR
                and actor.user_id not in self.contractor_contact_targets
                and actor.user_id not in self.contractor_contracts
            ):
                return False
            if actor.role == Role.GODFATHER and actor.user_id not in self.godfather_targets:
                return False
            if actor.role == Role.TERRORIST and actor.user_id not in self.terrorist_action_submitted:
                return False
        return True

    def spy_can_use_bonus_action(self, actor_id: int) -> bool:
        return self.phase == Phase.NIGHT and self._is_alive(actor_id) and actor_id in self.spy_bonus_pending

    def contractor_can_use_contract(self, actor_id: int) -> bool:
        actor = self.get_player(actor_id)
        return bool(
            self.phase == Phase.NIGHT
            and actor
            and actor.alive
            and actor.role == Role.CONTRACTOR
            and self.day_number >= 2
            and len(self.contractor_contract_targets(actor)) >= 2
        )

    def contractor_contract_targets(self, actor: Player) -> list[Player]:
        return [
            player
            for player in self.alive_players()
            if player.user_id != actor.user_id
            and player.user_id not in self.publicly_revealed_ids
        ]

    def all_day_votes_submitted(self) -> bool:
        if self.phase != Phase.VOTE:
            return False
        return all(player.user_id in self.day_votes for player in self.alive_players())

    def all_confirm_votes_submitted(self) -> bool:
        if self.phase != Phase.CONFIRM_VOTE:
            return False
        return all(player.user_id in self.confirm_votes for player in self.alive_players())

    def submit_night_action(self, actor_id: int, target_id: int) -> str:
        if self.phase != Phase.NIGHT:
            raise ValueError("지금은 밤이 아닙니다.")
        actor = self._require_alive(actor_id)

        if actor.role == Role.MAFIA:
            target = self._require_alive(target_id)
            if actor_id in self.mafia_targets:
                raise ValueError("이미 이번 밤 행동을 선택했습니다.")
            if not self.can_mafia_attack(target):
                raise ValueError("마피아는 접선된 마피아 팀을 공격 대상으로 고를 수 없습니다.")
            self.mafia_targets[actor_id] = target_id
            return f"공격 대상: {target.name}"

        if actor.role == Role.DOCTOR:
            target = self._require_alive(target_id)
            if actor_id in self.doctor_targets:
                raise ValueError("이미 이번 밤 행동을 선택했습니다.")
            self.doctor_targets[actor_id] = target_id
            return f"보호 대상: {target.name}"

        if actor.role == Role.POLICE:
            target = self._require_alive(target_id)
            if actor_id in self.police_targets:
                raise ValueError("이미 이번 밤 행동을 선택했습니다.")
            if actor_id == target_id:
                raise ValueError("경찰은 자기 자신을 조사할 수 없습니다.")
            self.police_targets[actor_id] = target_id
            return f"조사 투표 대상: {target.name}"

        if actor.role == Role.DETECTIVE:
            target = self._require_alive(target_id)
            if actor_id in self.detective_targets:
                raise ValueError("이미 이번 밤 행동을 선택했습니다.")
            if actor_id == target_id:
                raise ValueError("사립탐정은 자기 자신을 추적할 수 없습니다.")
            self.detective_targets[actor_id] = target_id
            return f"추적 대상: {target.name}"

        if actor.role == Role.SHAMAN:
            target = self._require_player(target_id)
            if actor_id in self.shaman_targets:
                raise ValueError("이미 이번 밤 행동을 선택했습니다.")
            if target.alive:
                raise ValueError("영매는 사망한 참가자만 성불할 수 있습니다.")
            if target.user_id in self.purified_dead_ids:
                raise ValueError("이미 성불한 사망자입니다.")
            self.shaman_targets[actor_id] = target_id
            return f"성불 대상: {target.name}"

        if actor.role == Role.SPY:
            target = self._require_alive(target_id)
            if self._spy_actions_used(actor_id) >= self._spy_action_limit(actor_id):
                raise ValueError("이미 이번 밤 행동을 선택했습니다.")
            if actor_id == target_id:
                raise ValueError("스파이는 자기 자신을 지목할 수 없습니다.")
            self.spy_targets.setdefault(actor_id, []).append(target_id)

            lines = [f"[첩보] {target.name} 님의 직업은 **{target.role.value}** 입니다."]
            if actor_id not in self.spy_contacted and target.role == Role.MAFIA:
                self.spy_contacted.add(actor_id)
                self.spy_bonus_pending.add(actor_id)
                self.spy_contacts_this_night.append(actor_id)
                lines.append("[접선] 마피아와 접선했습니다. 이번 밤에 한 번 더 첩보를 사용할 수 있습니다.")

            if actor_id in self.spy_bonus_pending and self._spy_actions_used(actor_id) >= 2:
                self.spy_bonus_pending.discard(actor_id)
            return "\n".join(lines)

        if actor.role == Role.TERRORIST:
            target = self._require_alive(target_id)
            if actor_id in self.terrorist_action_submitted:
                raise ValueError("이미 이번 밤 행동을 선택했습니다.")
            if actor_id == target_id:
                raise ValueError("테러리스트는 자기 자신을 지목할 수 없습니다.")
            self.terrorist_targets[actor_id] = target_id
            self.terrorist_action_submitted.add(actor_id)
            return f"지목 대상: {target.name}"

        if actor.role == Role.GODFATHER:
            target = self._require_alive(target_id)
            self.ensure_godfather_auto_contact()
            if actor_id not in self.godfather_contacted:
                raise ValueError("대부는 세 번째 밤부터 마피아 팀과 자동 접선되어 행동할 수 있습니다.")
            if actor_id in self.godfather_targets:
                raise ValueError("이미 이번 밤 행동을 선택했습니다.")
            if actor_id == target_id:
                raise ValueError("대부는 자기 자신을 지목할 수 없습니다.")
            self.godfather_targets[actor_id] = target_id
            return f"확정 처치 대상: {target.name}"

        raise ValueError(f"{actor.role.value}은/는 밤 행동이 없습니다.")

    def submit_contractor_contact(self, actor_id: int, target_id: int) -> str:
        if self.phase != Phase.NIGHT:
            raise ValueError("지금은 밤이 아닙니다.")
        actor = self._require_alive(actor_id)
        if actor.role != Role.CONTRACTOR:
            raise ValueError("청부업자만 동업을 사용할 수 있습니다.")
        if actor_id in self.contractor_contact_targets or actor_id in self.contractor_contracts:
            raise ValueError("이미 이번 밤 행동을 선택했습니다.")
        if actor_id in self.contractor_contacted:
            raise ValueError("이미 마피아와 접선했습니다.")

        target = self._require_alive(target_id)
        if actor_id == target_id:
            raise ValueError("청부업자는 자기 자신을 지목할 수 없습니다.")

        self.contractor_contact_targets[actor_id] = target_id
        if target.role == Role.MAFIA:
            self.contractor_contacted.add(actor_id)
            self.contractor_contacts_this_night.append(actor_id)
            return "[동업] 마피아와 접선했습니다."
        return "[동업] 접선에 실패했습니다."

    def submit_contractor_contract(
        self,
        actor_id: int,
        first_target_id: int,
        first_role: Role,
        second_target_id: int,
        second_role: Role,
    ) -> str:
        if self.phase != Phase.NIGHT:
            raise ValueError("지금은 밤이 아닙니다.")
        actor = self._require_alive(actor_id)
        if actor.role != Role.CONTRACTOR:
            raise ValueError("청부업자만 청부를 사용할 수 있습니다.")
        if self.day_number < 2:
            raise ValueError("청부는 두 번째 밤부터 사용할 수 있습니다.")
        if actor_id in self.contractor_contact_targets or actor_id in self.contractor_contracts:
            raise ValueError("이미 이번 밤 행동을 선택했습니다.")
        if first_target_id == second_target_id:
            raise ValueError("청부 대상 두 명은 서로 달라야 합니다.")
        if first_role not in CONTRACTOR_GUESSABLE_ROLES or second_role not in CONTRACTOR_GUESSABLE_ROLES:
            raise ValueError("청부로 추측할 수 없는 직업입니다.")

        first_target = self._require_alive(first_target_id)
        second_target = self._require_alive(second_target_id)
        if actor_id in {first_target_id, second_target_id}:
            raise ValueError("청부업자는 자기 자신을 지목할 수 없습니다.")
        if (
            first_target_id in self.publicly_revealed_ids
            or second_target_id in self.publicly_revealed_ids
        ):
            raise ValueError("직업이 공개적으로 드러난 사람은 청부 대상으로 지목할 수 없습니다.")

        self.contractor_contracts[actor_id] = (
            (first_target_id, first_role),
            (second_target_id, second_role),
        )
        return (
            "[청부] 암살 대상을 선택했습니다.\n"
            f"- {first_target.name}: {first_role.value}\n"
            f"- {second_target.name}: {second_role.value}"
        )

    def resolve_night(self) -> NightResult:
        if self.phase != Phase.NIGHT:
            raise ValueError("밤 단계만 정산할 수 있습니다.")

        self.ensure_godfather_auto_contact()
        mafia_target_id = self._majority_target(
            self.mafia_targets,
            self.alive_role_count(Role.MAFIA),
        )
        protected_id = self._majority_target(
            self.doctor_targets,
            self.alive_role_count(Role.DOCTOR),
        )
        police_target_id = self._majority_target(
            self.police_targets,
            self.alive_role_count(Role.POLICE),
        )
        godfather_attackers = {
            actor_id: target_id
            for actor_id, target_id in self.godfather_targets.items()
            if actor_id in self.godfather_contacted
        }
        godfather_target_id = self._majority_target(
            godfather_attackers,
            sum(
                1
                for player in self.alive_players()
                if player.role == Role.GODFATHER and player.user_id in self.godfather_contacted
            ),
        )

        mafia_target = self.get_player(mafia_target_id) if mafia_target_id else None
        protected = self.get_player(protected_id) if protected_id else None
        police_target = self.get_player(police_target_id) if police_target_id else None
        godfather_target = self.get_player(godfather_target_id) if godfather_target_id else None

        detective_results = self._resolve_detective_results(
            mafia_target_id,
            protected_id,
            police_target_id,
            godfather_target_id,
        )
        spy_results, spy_contacts = self._resolve_spy_results()
        contractor_results, contractor_contacts, contractor_kills = self._resolve_contractor_results()
        godfather_results = self._resolve_godfather_results()
        shaman_results, shaman_purifications = self._resolve_shaman_results()

        killed_players: list[Player] = []
        killed_by_mafia_team_ids: set[int] = set()
        soldier_blocks: list[Player] = []
        if mafia_target and mafia_target.alive and mafia_target.user_id != protected_id:
            if (
                mafia_target.role == Role.SOLDIER
                and mafia_target.user_id not in self.soldier_bulletproof_used
            ):
                self.soldier_bulletproof_used.add(mafia_target.user_id)
                self.publicly_revealed_ids.add(mafia_target.user_id)
                soldier_blocks.append(mafia_target)
            else:
                mafia_target.alive = False
                killed_players.append(mafia_target)
                killed_by_mafia_team_ids.add(mafia_target.user_id)
        if godfather_target and godfather_target.alive:
            godfather_target.alive = False
            if godfather_target not in killed_players:
                killed_players.append(godfather_target)
            killed_by_mafia_team_ids.add(godfather_target.user_id)

        for contractor_kill in contractor_kills:
            if contractor_kill.alive:
                contractor_kill.alive = False
            if contractor_kill not in killed_players:
                killed_players.append(contractor_kill)
            killed_by_mafia_team_ids.add(contractor_kill.user_id)

        terrorist_retaliations = self._resolve_terrorist_night_retaliations(
            killed_by_mafia_team_ids,
            killed_players,
        )

        graverobber_results = self._resolve_graverobbers(killed_players)

        self.mafia_targets.clear()
        self.doctor_targets.clear()
        self.police_targets.clear()
        self.detective_targets.clear()
        self.shaman_targets.clear()
        self.spy_targets.clear()
        self.spy_bonus_pending.clear()
        self.spy_contacts_this_night.clear()
        self.contractor_contact_targets.clear()
        self.contractor_contracts.clear()
        self.contractor_contacts_this_night.clear()
        self.godfather_targets.clear()
        self.terrorist_action_submitted.clear()
        self.day_votes.clear()
        self.confirm_votes.clear()
        self.phase = Phase.DAY
        return NightResult(
            killed=killed_players[0] if killed_players else None,
            protected=protected,
            mafia_target=mafia_target,
            police_target=police_target,
            police_target_is_mafia=(
                self.is_known_mafia_team(police_target) if police_target is not None else None
            ),
            killed_players=killed_players,
            detective_results=detective_results,
            spy_results=spy_results,
            spy_contacts=spy_contacts,
            contractor_results=contractor_results,
            contractor_contacts=contractor_contacts,
            contractor_kills=contractor_kills,
            godfather_results=godfather_results,
            godfather_contacts=[],
            graverobber_results=graverobber_results,
            terrorist_retaliations=terrorist_retaliations,
            soldier_blocks=soldier_blocks,
            shaman_results=shaman_results,
            shaman_purifications=shaman_purifications,
        )

    def ensure_godfather_auto_contact(self) -> list[int]:
        if self.day_number < 3:
            return []
        contacted: list[int] = []
        for player in self.alive_players():
            if player.role == Role.GODFATHER and player.user_id not in self.godfather_contacted:
                self.godfather_contacted.add(player.user_id)
                contacted.append(player.user_id)
        return contacted

    def start_vote(self) -> None:
        if self.phase != Phase.DAY:
            raise ValueError("낮 단계에서만 투표를 시작할 수 있습니다.")
        self.phase = Phase.VOTE
        self.day_votes.clear()
        self.confirm_votes.clear()

    def submit_day_vote(self, voter_id: int, target_id: int | None) -> str:
        if self.phase != Phase.VOTE:
            raise ValueError("지금은 투표 시간이 아닙니다.")
        voter = self._require_alive(voter_id)
        if target_id is None:
            self.day_votes[voter.user_id] = None
            return "투표 대상: 스킵"

        target = self._require_alive(target_id)
        self.day_votes[voter.user_id] = target.user_id
        return f"투표 대상: {target.name}"

    def resolve_nomination_vote(self) -> VoteResult:
        if self.phase != Phase.VOTE:
            raise ValueError("투표 단계만 정산할 수 있습니다.")

        live_votes = {
            voter_id: target_id
            for voter_id, target_id in self.day_votes.items()
            if self._is_alive(voter_id) and (target_id is None or self._is_alive(target_id))
        }
        if not live_votes:
            self._advance_to_next_night()
            return VoteResult(executed=None, tied=False, skipped=False)

        counts: Counter[int | None] = Counter()
        for voter_id, target_id in live_votes.items():
            counts[target_id] += self._vote_weight(voter_id)
        vote_counts = dict(counts)
        highest = max(counts.values())
        top_targets = [target_id for target_id, count in counts.items() if count == highest]
        if len(top_targets) != 1:
            self._advance_to_next_night()
            return VoteResult(executed=None, tied=True, skipped=False, vote_counts=vote_counts)

        if top_targets[0] is None:
            self._advance_to_next_night()
            return VoteResult(executed=None, tied=False, skipped=True, vote_counts=vote_counts)

        nominated = self.get_player(top_targets[0])
        self.phase = Phase.FINAL_DEFENSE
        return VoteResult(executed=nominated, tied=False, skipped=False, vote_counts=vote_counts)

    def resolve_vote(self) -> VoteResult:
        return self.resolve_nomination_vote()

    def start_confirmation_vote(self) -> None:
        if self.phase != Phase.FINAL_DEFENSE:
            raise ValueError("최후변론 뒤에만 찬반투표를 시작할 수 있습니다.")
        self.phase = Phase.CONFIRM_VOTE
        self.confirm_votes.clear()

    def submit_confirmation_vote(self, voter_id: int, approve: bool) -> str:
        if self.phase != Phase.CONFIRM_VOTE:
            raise ValueError("지금은 찬반투표 시간이 아닙니다.")
        voter = self._require_alive(voter_id)
        self.confirm_votes[voter.user_id] = approve
        return "찬성에 투표했습니다." if approve else "반대에 투표했습니다."

    def resolve_confirmation_vote(self, target_id: int) -> ConfirmVoteResult:
        if self.phase != Phase.CONFIRM_VOTE:
            raise ValueError("찬반투표 단계만 정산할 수 있습니다.")

        live_votes = {
            voter_id: approve
            for voter_id, approve in self.confirm_votes.items()
            if self._is_alive(voter_id)
        }
        counts: Counter[bool] = Counter()
        for voter_id, approve in live_votes.items():
            counts[approve] += self._vote_weight(voter_id)
        yes = counts.get(True, 0)
        no = counts.get(False, 0)
        tied = yes == no
        target = self.get_player(target_id)
        approved = target is not None and target.alive and yes > no
        blocked_by_politician = bool(approved and target and target.role == Role.POLITICIAN)
        executed = target if approved and not blocked_by_politician else None
        extra_killed: list[Player] = []
        if blocked_by_politician and target:
            self.publicly_revealed_ids.add(target.user_id)
        if executed:
            executed.alive = False
            if executed.role == Role.JOKER:
                self.joker_won = True
            if executed.role == Role.TERRORIST:
                terrorist_target = self.get_player(self.terrorist_targets.get(executed.user_id, 0))
                if (
                    terrorist_target
                    and terrorist_target.alive
                    and not self.is_citizen_team(terrorist_target)
                ):
                    terrorist_target.alive = False
                    extra_killed.append(terrorist_target)
        self._advance_to_next_night()
        return ConfirmVoteResult(
            executed=executed,
            approved=approved,
            tied=tied,
            blocked_by_politician=blocked_by_politician,
            extra_killed=extra_killed,
            vote_counts=dict(counts),
        )

    def winner(self) -> Winner | None:
        if self.joker_won:
            return Winner.JOKER
        mafia_alive = len(self.alive_known_mafia_team())
        citizen_alive = len(self.alive_players()) - mafia_alive
        if mafia_alive == 0:
            return Winner.CITIZEN
        if mafia_alive >= citizen_alive:
            return Winner.MAFIA
        return None

    def reveal_roles(self) -> str:
        return "\n".join(
            f"- {player.name}: {player.role.value}{'' if player.alive else ' (사망)'}"
            for player in sorted(self.players, key=lambda item: item.name.casefold())
        )

    def public_status(self) -> str:
        alive = ", ".join(player.name for player in self.alive_players())
        dead = ", ".join(player.name for player in self.dead_players()) or "없음"
        return (
            f"{self.day_number}일차 / 현재 단계: {self.phase.value}\n"
            f"생존자({len(self.alive_players())}명): {alive}\n"
            f"사망자: {dead}"
        )

    def _resolve_detective_results(
        self,
        mafia_target_id: int | None,
        protected_id: int | None,
        police_target_id: int | None,
        godfather_target_id: int | None,
    ) -> dict[int, str]:
        results: dict[int, str] = {}
        for actor_id, watched_id in self.detective_targets.items():
            actor = self.get_player(actor_id)
            watched = self.get_player(watched_id)
            if not actor or not actor.alive or not watched:
                continue

            action_target_id = self._resolved_action_target(
                watched,
                mafia_target_id,
                protected_id,
                police_target_id,
                godfather_target_id,
            )
            if action_target_id is None:
                results[actor_id] = f"{watched.name} 님은 밤에 능력을 사용하지 않았습니다."
                continue

            action_target = self.get_player(action_target_id)
            target_name = action_target.name if action_target else str(action_target_id)
            results[actor_id] = f"{watched.name} 님은 밤에 {target_name} 님에게 능력을 사용했습니다."
        return results

    def _resolved_action_target(
        self,
        watched: Player,
        mafia_target_id: int | None,
        protected_id: int | None,
        police_target_id: int | None,
        godfather_target_id: int | None,
    ) -> int | None:
        if watched.role == Role.MAFIA:
            return mafia_target_id
        if watched.role == Role.DOCTOR:
            return self.doctor_targets.get(watched.user_id)
        if watched.role == Role.POLICE:
            return police_target_id if watched.user_id in self.police_targets else None
        if watched.role == Role.DETECTIVE:
            return self.detective_targets.get(watched.user_id)
        if watched.role == Role.SHAMAN:
            return self.shaman_targets.get(watched.user_id)
        if watched.role == Role.SPY:
            targets = self.spy_targets.get(watched.user_id, [])
            return targets[-1] if targets else None
        if watched.role == Role.CONTRACTOR:
            if watched.user_id in self.contractor_contact_targets:
                return self.contractor_contact_targets.get(watched.user_id)
            contract = self.contractor_contracts.get(watched.user_id)
            return contract[0][0] if contract else None
        if watched.role == Role.TERRORIST:
            return self.terrorist_targets.get(watched.user_id)
        if watched.role == Role.GODFATHER:
            if watched.user_id in self.godfather_contacted:
                return godfather_target_id
            return self.godfather_targets.get(watched.user_id)
        return None

    def _resolve_terrorist_night_retaliations(
        self,
        killed_by_mafia_team_ids: set[int],
        killed_players: list[Player],
    ) -> list[tuple[Player, Player]]:
        retaliations: list[tuple[Player, Player]] = []
        for terrorist_id in killed_by_mafia_team_ids:
            terrorist = self.get_player(terrorist_id)
            if not terrorist or terrorist.role != Role.TERRORIST:
                continue

            target = self.get_player(self.terrorist_targets.get(terrorist_id, 0))
            if not target or not target.alive or not self.is_mafia_team(target):
                continue

            target.alive = False
            killed_players.append(target)
            retaliations.append((terrorist, target))
        return retaliations

    def _resolve_spy_results(self) -> tuple[dict[int, str], list[int]]:
        results: dict[int, str] = {}
        for actor_id, target_ids in self.spy_targets.items():
            actor = self.get_player(actor_id)
            if not actor or not actor.alive:
                continue
            lines: list[str] = []
            for target_id in target_ids:
                target = self.get_player(target_id)
                if target:
                    lines.append(f"[첩보] {target.name} 님의 직업은 **{target.role.value}** 입니다.")
            if actor_id in self.spy_contacts_this_night:
                lines.append("[접선] 마피아와 접선했습니다.")
            if lines:
                results[actor_id] = "\n".join(lines)
        return results, list(self.spy_contacts_this_night)

    def _resolve_contractor_results(self) -> tuple[dict[int, str], list[int], list[Player]]:
        results: dict[int, str] = {}
        kills: list[Player] = []

        for actor_id, target_id in self.contractor_contact_targets.items():
            actor = self.get_player(actor_id)
            target = self.get_player(target_id)
            if not actor or not actor.alive or not target:
                continue
            if target.role == Role.MAFIA and actor_id in self.contractor_contacts_this_night:
                results[actor_id] = "[동업] 마피아와 접선했습니다."
            else:
                results[actor_id] = "[동업] 접선에 실패했습니다."

        for actor_id, contract in self.contractor_contracts.items():
            actor = self.get_player(actor_id)
            if not actor or not actor.alive:
                continue

            targets = [
                (self.get_player(target_id), guessed_role)
                for target_id, guessed_role in contract
            ]
            success = all(
                target
                and target.alive
                and self.is_citizen_team(target)
                and target.role == guessed_role
                and target.user_id not in self.publicly_revealed_ids
                for target, guessed_role in targets
            )
            if not success:
                results[actor_id] = "대상의 정보가 정확하지 않아 암살에 실패했습니다."
                continue

            for target, _guessed_role in targets:
                if target and target not in kills:
                    kills.append(target)
            results[actor_id] = "청부가 성공했습니다. 대상 둘이 아침에 암살됩니다."

        return results, list(self.contractor_contacts_this_night), kills

    def _resolve_godfather_results(self) -> dict[int, str]:
        results: dict[int, str] = {}
        for actor_id, target_id in self.godfather_targets.items():
            actor = self.get_player(actor_id)
            target = self.get_player(target_id)
            if not actor or not actor.alive or not target or not target.alive:
                continue
            results[actor_id] = f"{target.name} 님을 확정 처치 대상으로 지목했습니다."
        return results

    def _resolve_shaman_results(self) -> tuple[dict[int, str], list[int]]:
        results: dict[int, str] = {}
        purifications: list[int] = []
        for actor_id, target_id in self.shaman_targets.items():
            actor = self.get_player(actor_id)
            target = self.get_player(target_id)
            if (
                not actor
                or not actor.alive
                or not target
                or target.alive
                or target.user_id in self.purified_dead_ids
            ):
                continue

            self.purified_dead_ids.add(target.user_id)
            purifications.append(target.user_id)
            results[actor_id] = (
                f"[성불] {target.name} 님의 직업은 **{target.role.value}** 입니다.\n"
                "대상은 사망자 채널에서 채팅할 수 없습니다."
            )
        return results, purifications

    def _resolve_graverobbers(self, killed_players: list[Player]) -> dict[int, Role]:
        if self.day_number != 1:
            return {}

        robbed_player = killed_players[0] if killed_players else None
        inherited_role = robbed_player.role if robbed_player else Role.CITIZEN
        results: dict[int, Role] = {}
        for player in self.alive_players():
            if player.role != Role.GRAVEROBBER:
                continue
            player.role = inherited_role
            results[player.user_id] = inherited_role
        if results and robbed_player:
            robbed_player.role = Role.VILLAIN if inherited_role in MAFIA_TEAM_ROLES else Role.CITIZEN
        return results

    def _require_alive(self, user_id: int) -> Player:
        player = self._require_player(user_id)
        if not player.alive:
            raise ValueError("사망한 참가자는 행동할 수 없습니다.")
        return player

    def _require_player(self, user_id: int) -> Player:
        player = self.get_player(user_id)
        if not player:
            raise ValueError("게임 참가자가 아닙니다.")
        return player

    def _is_alive(self, user_id: int) -> bool:
        player = self.get_player(user_id)
        return bool(player and player.alive)

    def _majority_target(self, targets: dict[int, int], voter_count: int) -> int | None:
        live_targets = [
            target_id
            for actor_id, target_id in targets.items()
            if self._is_alive(actor_id) and self._is_alive(target_id)
        ]
        if not live_targets or voter_count <= 0:
            return None
        counts = Counter(live_targets)
        highest = max(counts.values())
        tied = [target_id for target_id, count in counts.items() if count == highest]
        if len(tied) != 1:
            return None
        if highest <= voter_count / 2:
            return None
        return tied[0]

    def _spy_actions_used(self, actor_id: int) -> int:
        return len(self.spy_targets.get(actor_id, []))

    def _spy_action_limit(self, actor_id: int) -> int:
        return 2 if actor_id in self.spy_bonus_pending else 1

    def _contractor_can_act(self, player: Player) -> bool:
        alive_targets = [
            target for target in self.alive_players() if target.user_id != player.user_id
        ]
        if player.user_id not in self.contractor_contacted and alive_targets:
            return True
        return self.day_number >= 2 and len(self.contractor_contract_targets(player)) >= 2

    def _vote_weight(self, voter_id: int) -> int:
        voter = self.get_player(voter_id)
        if voter and voter.alive and voter.role == Role.POLITICIAN:
            return 2
        return 1

    def _advance_to_next_night(self) -> None:
        self.phase = Phase.NIGHT
        self.day_number += 1
