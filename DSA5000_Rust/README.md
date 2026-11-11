# Scanivalve DSA5000 SSEP Native Desktop Studio (v2.01)

Scanivalve 공식 매뉴얼(`DSA5000_V201_260723.pdf`, Software Version 2.01) 규격과 REA(Reverse Engineer Anything) 바이너리 분석 결과를 바탕으로, **웹 방식(브라우저)을 배제하고 100% 순수 Rust 기반 독립 실행형 윈도우 데스크톱 GUI 애플리케이션(Native Desktop GUI)**으로 완성된 고성능 SSEP 계측 소프트웨어입니다.

---

## 1. SSEP (Scanivalve Synchronous Ethernet Protocol) 규격 준수 구현

공식 매뉴얼 Section 3(Communications & Scanning) 및 Section 5(Software / SSEP Commands) 규격을 정밀하게 반영하였습니다:

### 1.1 SSEP 아키텍처 및 라인 토폴로지
* **데이지 체인 라인 토폴로지 (Daisy-Chain Line Topology)**:
  * 1대의 **Controller**(Index 0)와 최대 7대의 **Responder**(Index 1~7)로 구성되어 최대 8개 모듈(128 채널)을 동기화합니다.
  * 호스트 PC는 오직 단일 **Controller**의 Gigabit Ethernet 포트로만 연결하며, Responder 모듈들은 M8 4핀 SSEP 라인을 통해 Controller와 직렬로 상호 연결됩니다.
* **SSEP 다채널 동적 바이너리 패킷 (Table 5.1 & 5.2 규격)**:
  * 포트 503 바이너리 스트리밍은 SSEP 모듈 수에 따라 동적으로 가변되는 구조(`36 + N * 140` Bytes)를 가집니다:
    * **글로벌 헤더 (36 Bytes)**: `0x0200` (Packet ID) + `num_modules`(u16) + `frame_number`(u32) + IEEE-1588 PTP 초/나노초(u32 x 2) + Spare(20B)
    * **모듈 데이터 블록 (140 Bytes × N 모듈)**: 모듈 타입/SN + 응답기 주소(0=Ctrl, 1..7=Resp) + 상태 플래그 + **16채널 압력 (16 × f32)** + **16채널 온도 (16 × f32)**
  * 1모듈(16ch: 176B), 2모듈(32ch: 316B), 4모듈(64ch: 596B), 8모듈(128ch: 1156B)을 실시간 무손실 파싱합니다.
* **SSEP 동기화 제어 및 영점 교정**:
  * `LIST SYS *`: SSEP 네트워크에 연결된 전체 모듈의 Index, S/N, 모델, 압력 범위(NPR), 펌웨어 버전, 밸브 상태, IP 등을 자동 스캔합니다.
  * `SMODE`: 현재 모듈의 역할(`Controller`, `Responder`, `Standalone`), Responder 개수, SSEP In/Out 링크 상태(`UP`/`DOWN`)를 진단합니다.
  * `VALVE <PX|CAL|PRG|ISO>`: Controller에 명령을 송신하면 전체 연결된 Responder들의 공압 솔레노이드 밸브가 일괄 동기화 전환됩니다.
  * `CALZ`: Controller를 통해 모든 SSEP 모듈의 16~128개 압력 센서 영점 보정이 일괄 수행됩니다.

---

## 2. 독립 실행형 네이티브 데스크톱 GUI (Zero Browser)

웹 브라우저를 띄우지 않고, 더블 클릭 즉시 OS 자체 네이티브 창(`HWND`)으로 렌더링되는 독립 실행 GUI 프로그램입니다.

* **GPU 하드웨어 가속 60 FPS 렌더링 (`eframe` & `egui_plot`)**:
  * 웹 기반 렌더링의 오버헤드 없이 그래픽 카드를 직접 활용하여 100Hz~5,000Hz 고속 스트리밍에서도 부드러운 오실로스코프 파형을 그립니다.
* **완전한 독립 바이너리 (`DSA5000_Studio.exe`)**:
  * 추가 DLL 설치, LabVIEW Run-Time, WebView2, 웹 브라우저 등이 전혀 필요 없는 100% 완전 독립 실행 파일입니다.
* **소켓 잠김(Lockup) 원천 방지**:
  * 윈도우 창 닫기 또는 비정상 종료 시 RAII `Drop` 가드가 Port 503으로 STOP(`0`) 신호를 즉각 송출하고 소켓을 반환하여 하드웨어 포트 락업을 방지합니다.

---

## 3. UI 주요 기능 탭 안내

1. **📊 실시간 오실로스코프 (Multi-Module Oscilloscope)**:
   * SSEP 모듈별 탭 필터링 (`전체 채널`, `모듈 0 (Ch 1-16)`, `모듈 1 (Ch 17-32)`, `모듈 2 (Ch 33-48)` 등)
   * 16개 고유 채널 색상 칩 클릭으로 개별 채널 실시간 ON/OFF 토글
   * 시간축 윈도우 조절 (1s ~ 30s), Y축 자동 스케일링, 파형 일시 정지(Pause) 및 버퍼 초기화
2. **📶 채널별 바 미터 (Dynamic Bar Gauges)**:
   * SSEP 전체 모듈 및 채널의 순시 압력 레벨을 고밀도 컬러 바 게이지와 수치로 시각화
3. **📈 실시간 통계 분석 (Real-Time Statistics)**:
   * 16~128개 전체 채널별 Current, Mean, Min, Max, Peak-to-Peak, RMS(Noise), 센서 온도(°C) 실시간 연산
4. **🔗 SSEP 토폴로지 및 장비 진단 (Network Topology)**:
   * Host PC에서 Controller, Responder로 이어지는 SSEP 직렬 라인 토폴로지 그래픽 표시
   * `LIST SYS *` 장비 상세 사양 테이블 및 `SMODE`, `SSEP 0(Auto)`, `SSEP 2(Controller)` 원클릭 진단
5. **💾 CSV 데이터 로깅 (Scanivalve Standard Logger)**:
   * Scanivalve 공식 표준 메타데이터 헤더와 IEEE-1588 PTP 정밀 타임스탬프가 포함된 멀티모듈 CSV 무손실 저장
6. **💻 Telnet 콘솔 터미널 (Port 23)**:
   * Port 23 Telnet ASCII 명령 직접 송수신 및 `STATUS`, `LIST SYS *`, `SMODE`, `CALZ`, `VER ALL` 퀵 버튼 지원

---

## 4. 실행 방법

### 방법 1: 독립 실행 파일 직접 실행 (더블 클릭)
* **[`DSA5000_Studio.exe`](file:///C:/agy/dsa5k/DSA5000_Rust/DSA5000_Studio.exe)**를 더블 클릭합니다.
* 즉시 네이티브 데스크톱 GUI 창이 열립니다.
* 상단의 **[연결 (Connect)]** 버튼을 누르면 내부 SSEP 하드웨어 시뮬레이터가 기동되며 실시간 데이터 수집(SCAN)이 자동으로 시작되어 오실로스코프 파형과 바 미터가 즉시 동작합니다.
* 모듈 수 드롭다운에서 **1, 2, 3, 4, 5, 6, 7, 8 모듈 (16~128 채널)**을 자유롭게 선택하여 다양한 멀티 모듈 SSEP 계측 환경을 테스트할 수 있습니다.
* 실제 물리 장비에 연결할 때는 **"시뮬레이터 모드"** 체크를 해제하고 Controller IP(기본: `192.168.1.100`)를 입력한 후 연결하시면 됩니다.

### 방법 2: 배치 파일 메뉴 실행
* **[`run_dsa5000_rust.bat`](file:///C:/agy/dsa5k/DSA5000_Rust/run_dsa5000_rust.bat)**을 더블 클릭하여 실행 모드를 선택할 수 있습니다.

### 방법 3: 명령줄(CLI) 실행 옵션
```bash
# 1. 릴리즈 빌드
cargo build --release

# 2. SSEP 통합 무결성 자동 검증 테스트 (100% 통과 검증)
cargo run --release -- --test

# 3. CLI 콘솔 대시보드 모드 실행
cargo run --release -- --cli --simulate
```

---

## 5. 프로젝트 소스 코드 구성

* [`src/main.rs`](file:///C:/agy/dsa5k/DSA5000_Rust/src/main.rs): 네이티브 윈도우 GUI 엔트리포인트 (`eframe::run_native`) 및 CLI 모드
* [`src/gui.rs`](file:///C:/agy/dsa5k/DSA5000_Rust/src/gui.rs): `egui` 및 `egui_plot` 기반 60 FPS SSEP 데스크톱 GUI 구현
* [`src/protocol.rs`](file:///C:/agy/dsa5k/DSA5000_Rust/src/protocol.rs): SSEP 다중 모듈 바이너리 패킷 파서 (`36 + N*140` Bytes) 및 SSEP 구조체
* [`src/client.rs`](file:///C:/agy/dsa5k/DSA5000_Rust/src/client.rs): 이중 TCP 소켓 클라이언트, SSEP 장비 디스커버리 (`query_ssep_system`), RAII 소켓 해제
* [`src/simulator.rs`](file:///C:/agy/dsa5k/DSA5000_Rust/src/simulator.rs): 멀티모듈(1~8대) SSEP 하드웨어 시뮬레이터 (동기화 파형 및 `LIST SYS *` 에뮬레이션)
* [`src/logger.rs`](file:///C:/agy/dsa5k/DSA5000_Rust/src/logger.rs): 전체 SSEP 채널을 기록하는 고속 버퍼링 CSV 로거
