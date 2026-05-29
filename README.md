# Discord 마피아 봇

Discord 서버에서 슬래시 명령어로 실행하는 마피아 게임 봇입니다.

## 사용 방법

1. Python 3.11+ 환경에서 의존성을 설치합니다.

```bash
pip install -r requirements.txt
```

2. `.env.example`을 `.env`로 복사하고 봇 토큰을 넣습니다.

```env
DISCORD_TOKEN=your_bot_token_here
```

3. Discord Developer Portal에서 봇의 `Server Members Intent`를 켜고, 서버에 `bot` + `applications.commands` 스코프로 초대합니다.

4. 서버에 아래 역할을 만들어 둡니다.

- `마피아 참가자`
- `관리자`
- `게임알림` (선택)

5. 봇을 실행합니다.

```bash
python bot.py
```

## 주요 명령어

```text
/마피아시작
/마피아설정
/마피아능력
/역할안내
/마피아상태
/마피아중지
```

## 설정 파일

- `config.json`에서 기본 직업 수, 시간, 역할 이름 등을 조정할 수 있습니다.
