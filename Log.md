# Log — <프로젝트명>

> append-only. 기존 줄은 수정·삭제하지 않는다. 새 항목은 항상 파일 끝에.
> 형식: `## YYYY-MM-DD HH:MM — <작업 제목>` + [DONE] 블록

---

## 2026-11-06 22:18 — 스탠드얼론 모드 다중 장치 동시 계측 기능 추가

[DONE]
1. 수정 파일:
   - DSA5000_Rust/src/client.rs
   - DSA5000_Rust/src/simulator.rs
   - DSA5000_Rust/src/gui.rs
   - DSA5000_Rust/src/main.rs
2. 검증: `.\DSA5000_Studio.exe --test` → 7개 통합 테스트(SSEP 1~5 + 단일 스탠드얼론 6/7 + 멀티 스탠드얼론 7/7 32채널) 100% 통과
3. 미해결 / 제안: 없음

## 2026-11-07 04:54 — 다중 장치 동시(Broadcast) 및 개별(Targeted) 제어 기능 추가

[DONE]
1. 수정 파일:
   - DSA5000_Rust/src/client.rs
   - DSA5000_Rust/src/gui.rs
   - DSA5000_Rust/src/main.rs
2. 검증: `.\DSA5000_Studio.exe --test` → 7개 통합 테스트(개별 밸브 제어, 동시 밸브 제어, 개별/동시 CALZ, 32채널 스트리밍) 100% 통과
3. 미해결 / 제안: 없음

## 2026-11-07 05:06 — 모드 변경 시 SSEP 명령어(SSEP 0 / SSEP 1 및 SAVE) 연동 구현

[DONE]
1. 수정 파일:
   - DSA5000_Rust/src/simulator.rs
   - DSA5000_Rust/src/client.rs
   - DSA5000_Rust/src/gui.rs
   - DSA5000_Rust/src/main.rs
2. 검증: `.\DSA5000_Studio.exe --test` → 7개 통합 테스트(SSEP 0 / SSEP 1 모드 설정 및 상태 확인, 개별/동시 밸브 및 CALZ 제어) 100% 통과
3. 미해결 / 제안: 없음

## 2026-11-07 05:14 — CSV 데이터 저장 단일 파일 통합 모드(Single Unified File) 구현

[DONE]
1. 수정 파일:
   - DSA5000_Rust/src/logger.rs
   - DSA5000_Rust/src/gui.rs
   - DSA5000_Rust/src/main.rs
2. 검증: `.\DSA5000_Studio.exe --test` → 7개 통합 테스트(멀티 디바이스 32채널 전체의 단일 CSV 파일 통합 저장 검증 포함) 100% 통과
3. 미해결 / 제안: 없음


## 2026-11-07 05:27 — 장치 수신 PTP 데이터(타임스탬프 및 지터) 계측 화면 실시간 표시

[DONE]
1. 수정 파일:
   - DSA5000_Rust/src/client.rs : ClientStats 구조체 및 패킷 파싱 루프에 last_ptp_sec, last_ptp_nsec 필드 및 지터 반영
   - DSA5000_Rust/src/gui.rs : DsaApp 구조체, 패킷 드레인, format_ptp_display 함수 및 계측 화면(상단 HUD, 하단 상태바, 오실로스코프 제어줄·범례, 바미터, 통계 테이블)에 실시간 PTP 표시
2. 검증: `.\DSA5000_Studio.exe --test` → 7개 자동 통합 테스트 100% 통과 (PTP 타이밍 스트리밍 및 다채널 데이터 수신 정상)
3. 미해결 / 제안: 없음


## 2026-11-09 11:20 — 실기기 2대(191.30.130.5, 191.30.130.6) 스탠드얼론 명령 및 계측 통신 최적화

[DONE]
1. 수정 파일:
   - DSA5000_Rust/src/client.rs : 실기기 Telnet 명령 소켓 타임아웃 단축(200ms), 설정 명령 시 지연 방지 및 확인 쿼리 연동, 연결 및 스캔 시작 시 FPS 0(무제한 연속 스트리밍) 기본값 보장
2. 검증: `.\DSA5000_Studio.exe --device 191.30.130.5,191.30.130.6` → 실기기 2대(32채널) 192프레임 연속 스트리밍 100% 정상 수신 검증
3. 미해결 / 제안: 없음


## 2026-11-09 11:42 — PTP 하단 단독 표시, 멈춤 제거, Y스케일 채움, 새로고침 보존, 수동 스캔 분리

[DONE]
1. 수정 파일:
   - DSA5000_Rust/src/client.rs : query_ssep_system 다중 장비 보존 및 SSEP 설정 시 불필요한 SAVE(EEPROM 쓰기) 제거
   - DSA5000_Rust/src/logger.rs : 파일 잠금 시 대체 인덱스 파일 자동 생성으로 REC 시작 실패 방지
   - DSA5000_Rust/src/gui.rs : PTP/지터 하단 상태바 단독 표시로 정리, Y축 자동 스케일 그래프 높이 92% 전체 채움 개선, 장치 새로고침 truncate 제거, 연결 시 자동 스캔 제거 및 SCAN 버튼 수동 시작 적용
2. 검증: `.\DSA5000_Studio.exe --test` → 7개 통합 테스트 100% 통과, `.\DSA5000_Studio.exe --device 191.30.130.5,191.30.130.6` → 실기기 2대 964프레임 연속 스트리밍 100% 정상 수신 검증
3. 미해결 / 제안: 없음


## 2026-11-09 12:05 — SSEP 모드 설정 시 Controller 및 Responder 명시적 지정 지원

[DONE]
1. 수정 파일:
   - DSA5000_Rust/src/client.rs : DeviceEndpoint 구조체에 SSEP 역할(ssep_role: SsepAssignment) 필드 및 생성자 헬퍼(with_role) 추가
   - DSA5000_Rust/src/gui.rs : SSEP 역할 명시적 지정(Controller=2, Responder=3, Last Responder=4, Auto=0, Standalone=1) UI, 상단 헤더바 역할 콤보박스 및 Flash SAVE 버튼, SSEP 토폴로지 탭 명시적 역할 지정 제어판, 다중 장치 관리자 행별 SSEP 역할 콤보박스 및 일괄 체인 설정/SAVE 기능 구현
   - DSA5000_Rust/src/main.rs : DeviceEndpoint 인스턴스화 호환성 반영 및 SSEP 0, 1, 2, 3, 4와 SAVE 하드웨어 명령 자동 통합 검증 추가
2. 검증:
   - `.\DSA5000_Studio.exe --test` → 7개 자동 통합 테스트 100% 통과 (SSEP 0, 2, 3, 4, SAVE 및 1 하드웨어 명령 검증 완료)
   - `.\DSA5000_Studio.exe --device 191.30.130.5,191.30.130.6` → 실기기 2대(32채널) 962프레임 연속 스트리밍 100% 정상 수신 검증
3. 미해결 / 제안: 없음


## 2026-11-09 12:18 — 채널 이름 설정 Gain/Offset 추가 및 실시간 통계분석 설정값·계산값 표시

[DONE]
1. 수정 파일:
   - DSA5000_Rust/src/gui.rs : ChannelStats 구조체에 Gain/Offset 계산 메서드(calc_value, calc_mean, calc_min, calc_max, calc_pp, calc_rms) 추가, AppConfig 및 DsaApp에 채널별 channel_gains/channel_offsets 필드 및 dsa5000_config.json 영속화, 패킷 드레인 루프 실시간 calc_current 계산 반영, 채널 설정창에 개별 Gain/Offset DragValue 및 일괄 적용 툴 추가, 실시간 통계 분석 탭에 Gain/Offset 설정값 및 계산값(Calc, Calc Mean, Min, Max, RMS) 칼럼 표시와 통계 초기화 버튼 추가
   - DSA5000_Rust/src/main.rs : 자동 테스트(--test)에 ChannelStats Gain/Offset 보정 계산식 단위 검증 추가
2. 검증:
   - `.\DSA5000_Studio.exe --test` → 8개 자동 통합 테스트 100% 통과 (Gain/Offset 보정 계산 검증 완료)
   - `.\DSA5000_Studio.exe --device 191.30.130.5,191.30.130.6` → 실기기 2대(32채널) 1930프레임 연속 스트리밍 100% 정상 수신 검증
3. 미해결 / 제안: 없음


## 2026-11-09 12:47 — 상단 헤더 HUD 제거 및 공압밸브 제어부 시스템 설정창 이전

[DONE]
1. 수정 파일:
   - DSA5000_Rust/src/gui.rs : 상단 헤더바에서 중복 표시되던 Rate/FPS/Frames/Loss HUD 제거, 리본 영역에서 공압밸브 제어 그룹 제거하고 시스템 설정창(render_scan_params_window)으로 이전하여 대상 모듈 선택 및 밸브 모드(Px/Cal/Prg/Iso) 제어 통합
2. 검증:
   - `.\DSA5000_Studio.exe --test` → 8개 자동 통합 테스트 100% 통과
   - `.\DSA5000_Studio.exe --device 191.30.130.5,191.30.130.6` → 실기기 2대(32채널) 1928프레임 연속 스트리밍 100% 정상 수신 검증
3. 미해결 / 제안: 없음


## 2026-11-09 15:26 — SSEP Controller 및 Responder 명시적 역할 지정 및 Flash SAVE 구현

[DONE]
1. 수정 파일:
   - DSA5000_Rust/src/client.rs : start_scan에서 SSEP Controller로부터 다중 모듈 패킷 수신 시 전체 모듈 보존 및 다중 엔드포인트 수집 지원
   - DSA5000_Rust/src/gui.rs : render_header_bar, render_device_manager_window, render_ssep_topology_tab, handle_connect, handle_apply_designated_ssep_roles, handle_apply_and_save_ssep_role : SSEP 모드 설정 시 각 장치별 Controller(2), Responder(3, 4), Auto(0), Standalone(1) 명시적 지정 UI 및 개별/일괄 Flash SAVE 기능 구현
2. 검증:
   - `cargo build --release --manifest-path C:\agy\dsa5k\DSA5000_Rust\Cargo.toml` → 컴파일 0 에러 완료
   - `.\DSA5000_Studio.exe --test` → 8개 자동 통합 테스트 100% 통과 (SSEP 토폴로지, 밸브 제어, CALZ, 503 바이너리 스트리밍, 단독/다중 모드, SSEP 0/1/2/3/4 및 SAVE 명령, 통계 분석)
   - `.\DSA5000_Studio.exe --device 191.30.130.5,191.30.130.6` → 실기기 2대(32채널) 13,420프레임 무손실 연속 수신 검증 완료
3. 미해결 / 제안: 없음


## 2026-11-09 16:35 — 영점 교정(CALZ) 및 0.000 PSI 소프트웨어 영점 보정(Zero Tare/Reset) 연동

[DONE]
1. 수정 파일:
   - DSA5000_Rust/src/client.rs : zero_calibration_target 실행 완료 시 하드웨어 Flash EEPROM에 DELTA 영점 테이블을 영구 저장(SAVE)하도록 명령 추가
   - DSA5000_Rust/src/gui.rs : handle_calz_target의 중복 소켓 생성 오류 제거 및 활성 연결을 통한 직접 CALZ 전송, 비차단 타이밍 제어 및 애니메이션 프로그레스 모달 다이얼로그(render_calz_modal) 구현, 하단 상태바 교정 진행률 표시, 소프트웨어 0점 보정(zero_software_offsets, handle_zero_tare, handle_reset_offsets) 추가, drain_incoming_packets 파형 버퍼 및 바미터·범례·통계 테이블 전체 0.000 PSI 실시간 연동, 상단 리본·채널설정·SSEP토폴로지에 ⚡ 즉각 영점(Tare) 및 리셋 버튼 추가
2. 검증:
   - `cargo build --release --manifest-path C:\agy\dsa5k\DSA5000_Rust\Cargo.toml` → 0 에러 빌드 성공
   - `.\DSA5000_Studio.exe --test` → 8개 자동 통합 테스트 100% 통과 (SSEP 토폴로지, 밸브 제어, CALZ 영점 교정, 503 바이너리 스트리밍, 단독/다중 모드, SSEP 0/1/2/3/4 및 SAVE 명령, 통계 분석)
   - `.\DSA5000_Studio.exe --device 191.30.130.5,191.30.130.6` → 실기기 2대(32채널) 1,929프레임 연속 무손실 스트리밍 100% 정상 수신 검증
3. 미해결 / 제안: 없음


## 2026-11-09 18:05 — CSV 저장 파일명 타임스탬프(년월일시분초), 그래프 X/Y/전체 줌, 플롯 두께 조절, 커서 위치 계측값 표시

[DONE]
1. 수정 파일:
   - DSA5000_Rust/src/logger.rs : current_timestamp_str 함수(Win32 GetLocalTime 무의존성 연동) 추가 및 CSV 저장 파일명에 '년월일시분초(YYYYMMDD_HHMMSS)' 자동 결합
   - DSA5000_Rust/src/gui.rs : AppConfig 및 DsaApp에 plot_line_width(기본 1.0px), y_zoom_factor, show_cursor_info, cursor_pos 필드 추가 및 영속화, render_oscilloscope_tab에 X축/Y축/전체 확대·축소·리셋 줌 툴바 및 플롯 선 두께 프리셋 조절기 추가, 마우스 오버 시 플롯 내 실시간 십자선(Crosshair) 및 좌표 배지 표시, 상단 실시간 커서 위치 시간/압력 및 채널별 계측값 HUD 배너 구현, render_data_logger_tab에 타임스탬프 포함 파일명 미리보기 연동
2. 검증:
   - `cargo build --release --manifest-path C:\agy\dsa5k\DSA5000_Rust\Cargo.toml` → 0 에러 컴파일 및 `DSA5000_Studio.exe` 갱신 배포 완료
   - `.\DSA5000_Studio.exe --test` → 8개 자동 통합 테스트 100% 통과 (타임스탬프 CSV 파일명 생성 SSEP_SelfTest_20261109_180014.csv 및 MultiDevice_Unified_Single_20261109_180018.csv 생성/기록 정상 검증)
   - `.\DSA5000_Studio.exe --device 191.30.130.5,191.30.130.6` → 실기기 2대(32채널) 1,927프레임 연속 무손실 스트리밍 100% 정상 수신 검증
3. 미해결 / 제안: 없음


## 2026-11-09 18:25 — CSV 저장 데이터 상대시간(Relative_Time_sec) 추가, 하단 독립 커서 계측 패널 분할 및 SSEP 매뉴얼 규격(0~4) 완벽 준수 검증

[DONE]
1. 수정 파일:
   - DSA5000_Rust/src/logger.rs : PTP 타임스탬프(ptp_sec + ptp_nsec) 및 고정밀 Instant 기반 상대시간(Relative_Time_sec, 마이크로초 6자리 정밀도) 필드 추가 및 CSV 헤더/데이터에 기록
   - DSA5000_Rust/src/gui.rs : 오실로스코프 그래프 하단에 독립된 커서 계측 전용 패널(Frame)을 분할 배치하여 커서 위치의 상대시간(X), 커서압력(Y) 및 전체 활성 채널별 계측값(가로 스크롤 카드 형태) 실시간 표출, Stroke::new(1.0_f32) 부동소수점 경고 제거
   - DSA5000_Rust/src/protocol.rs, client.rs, simulator.rs : 스캔밸브 SSEP 규격(0=Automatic, 1=Standalone, 2=Controller, 3=Responder, 4=Last Responder) 매뉴얼 사양과 100% 일치 확인 및 테스트 검증
2. 검증:
   - `cargo build --release --manifest-path C:\agy\dsa5k\DSA5000_Rust\Cargo.toml` → 0 에러 릴리즈 빌드 완료
   - `.\DSA5000_Studio.exe --test` → 8개 자동 통합 테스트 100% 통과 (SSEP 0, 1, 2, 3, 4 설정 및 EEPROM SAVE, 단독/다중 모드, 상대시간 CSV 통합 기록 등)
   - `.\DSA5000_Studio.exe --device 191.30.130.5,191.30.130.6` → 실기기 2대(32채널) 1400프레임 무손실 연속 수신 검증 완료
3. 미해결 / 제안: 없음


## 2026-11-09 18:40 — 커서 계측값 우측 세로 목록 연동, 하단 영역 제거 및 플롯 위 노란색 텍스트 제거

[DONE]
1. 수정 파일:
   - DSA5000_Rust/src/gui.rs : 오실로스코프 탭 하단 분할 영역(panel_frame) 완전 제거 및 그래프 전체 높이 복원, 플롯 위 노란색 커서 좌표 텍스트 제거하고 은은한 Cyan 십자선 유지, 일시 정지(Pause) 또는 계측 정지 상태에서 그래프 위 커서 이동 시 우측 세로 채널 목록(실시간 값 컬럼)에 해당 커서 시점의 전체 채널 계측값 실시간 연동 및 Cyan 하이라이트 표시
2. 검증:
   - `cargo build --release --manifest-path C:\agy\dsa5k\DSA5000_Rust\Cargo.toml` → 0 에러 빌드 성공
   - `.\DSA5000_Studio.exe --test` → 8개 자동 통합 테스트 100% 통과
   - `.\DSA5000_Studio.exe --device 191.30.130.5,191.30.130.6` → 실기기 2대(32채널) 1,404프레임 연속 스트리밍 100% 정상 수신 검증
3. 미해결 / 제안: 없음


## 2026-11-11 16:17 — PTP 고유 타임스탬프 기반 파형 수직선/겹침 왜곡 제거, 마우스 휠 축 선택식(전체/X/Y) 줌 개선 및 명령 대기 시 Progress 마우스 커서 표시

[DONE]
1. 수정 파일:
   - DSA5000_Rust/src/gui.rs : DsaApp 구조체, drain_incoming_packets, render_oscilloscope_tab, update, render_bottom_status_bar 및 명령 핸들러
2. 검증:
   - `cargo build --release` → 0 에러 빌드 성공 및 `DSA5000_Studio.exe` 갱신 배포 완료
   - `.\DSA5000_Studio.exe --test` → 8개 자동 통합 테스트 100% 통과 (PTP 패킷 스트리밍, SSEP 0~4 설정/SAVE, 멀티모듈 데이터 무손실 검증)
   - `.\DSA5000_Studio.exe --device 191.30.130.5,191.30.130.6` → 실기기 2대(32채널) 1,401프레임 무손실 연속 스트리밍 100% 정상 수신 검증
3. 미해결 / 제안: 없음


## 2026-11-11 16:44 — 줌 제어 확대/축소 버튼군 제거, 5초 이상 스케일 플롯 떨림 해소 및 명령 대기 시 Win32 IDC_WAIT/egui Wait 마우스 커서 연동

[DONE]
1. 수정 파일:
   - DSA5000_Rust/src/gui.rs : 줌 제어 툴바 확대/축소 버튼군 제거, 8000포인트 원본 직결 및 고안정 Min-Max 버킷팅으로 5초 이상 스케일 떨림 해소, Win32 SetCursor(IDC_WAIT) 및 egui Wait 마우스 진행중 커서 연동
2. 검증:
   - `cargo build --release` → 0 에러 빌드 성공 및 `DSA5000_Studio.exe` 갱신 배포 완료
   - `.\DSA5000_Studio.exe --test` → 8개 자동 통합 테스트 100% 통과 (PTP 패킷 스트리밍, SSEP 0~4 설정/SAVE, 멀티모듈 데이터 무손실 검증)
   - `.\DSA5000_Studio.exe --device 191.30.130.5,191.30.130.6` → 실기기 2대(32채널) 1,401프레임 무손실 연속 스트리밍 100% 정상 수신 검증
3. 미해결 / 제안: 없음


## 2026-11-11 19:15 — Git 로컬 리포지토리 초기화, .gitignore 작성 및 GitHub 원격(https://github.com/jaesang-jeong/dsa5k) 연동 초기 커밋 완료

[DONE]
1. 수정 파일:
   - .gitignore : 빌드 산출물(target/, *.exe), 대용량 zip(vdd/*.zip), 계측 로그(data_records/, *.csv), 로컬 설정 제외 설정
2. 검증:
   - `git init`, `git branch -M main`, `git commit` → 52개 파일(14,258줄) 커밋 완료 (2c53d93)
   - `git remote add origin https://github.com/jaesang-jeong/dsa5k.git` → 원격 저장소 연동 완료
3. 미해결 / 제안:
   - GitHub 원격 푸시 인증(Personal Access Token 입력 또는 대화형 터미널에서 git push -u origin main 실행)


## 2026-11-11 19:23 — GitHub 원격 저장소(https://github.com/jaesang-jeong/dsa5k) main 브랜치 푸시 완료

[DONE]
1. 수정 파일:
   - .gitignore : 빌드 산출물 및 불필요 파일 제외 설정
2. 검증:
   - `git push -u origin main` → `https://github.com/jaesang-jeong/dsa5k.git` (main -> main) 푸시 성공 (100% 완료)
3. 미해결 / 제안: 없음
