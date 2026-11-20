---
id: BOT-03
title: 오늘 배정 문제의 풀이 확인 및 알림
priority: P0
depends: [BOT-02]
policies: [bot-runtime]
design: false
---
오늘 배정한 문제별 해결 여부를 확인하고 남은 문제만 설정한 시각에 Discord로 알린다.
전부 해결했으면 독촉을 보내지 않는다. 외부 조회 실패는 확인 대기로 처리한다.
봇을 재시작하거나 스케줄러가 다시 실행될 때 같은 알림의 중복을 줄일 방법을 정한다.
