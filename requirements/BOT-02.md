---
id: BOT-02
title: 티어에 맞는 일일 문제 배정
priority: P0
depends: [BOT-01]
policies: [bot-runtime]
design: false
---
설정한 하루 문제 수만큼 현재 티어에 맞는 미해결 문제를 배정한다.
같은 날 다시 조회하면 저장된 배정과 제출 링크를 보여준다.
적합한 문제가 부족할 때 난이도를 얼마나 넓힐지와 부족분을 어떻게 표시할지 스펙에서 정한다.
