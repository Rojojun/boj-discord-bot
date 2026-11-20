---
id: BOT-06
title: 어려운 문제의 선수 학습 및 대체 문제 추천
priority: P1
depends: [BOT-02, BOT-05]
policies: [bot-runtime, model-runtime]
design: false
---
사용자가 오늘 배정 문제에서 막혔다고 하면 선수 개념, 선수 문제, 같은 주제의 더 쉬운 대체 문제를 제공한다.
사용자가 대체 문제를 선택하면 해당 배정 문제를 바꾸고 풀이 확인에도 반영한다.
선수 학습 문제의 완료가 오늘 과제와 커리큘럼 진행에 어떤 영향을 주는지 정한다.
