---
id: BOT-04
title: 학습 스킵 및 재개 질문
priority: P0
depends: [BOT-02, BOT-03]
policies: [bot-runtime]
design: false
---
사용자는 오늘 하루 또는 최대 7일까지 학습을 스킵할 수 있다.
스킵 중에는 새 배정과 독촉을 보내지 않는다. 기간이 끝나면 Discord에서 학습을 다시 시작할지 묻는다.
중간 재개가 가능하며 스킵과 재개 대기 상태는 재시작 후에도 유지된다.
