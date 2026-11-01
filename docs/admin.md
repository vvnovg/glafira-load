# Glafira for Load Testing: руководство администратора

## Установка
| ОС | Файл | Тихая установка |
|---|---|---|
| macOS | `Glafira_for_Load_Testing_<версия>_aarch64.dmg` / `_x64.dmg` | `hdiutil attach <файл>.dmg`, затем `sudo cp -R "/Volumes/Glafira for Load Testing/Glafira for Load Testing.app" /Applications/` и `hdiutil detach` |
| Windows | `Glafira_for_Load_Testing_<версия>_x64_en-US.msi` | `msiexec /i <файл>.msi /qn` (SCCM, Intune) — установка на машину |
| Linux | `Glafira_for_Load_Testing_<версия>_amd64.deb`, `Glafira_for_Load_Testing-<версия>-1.x86_64.rpm`, `Glafira_for_Load_Testing_<версия>_amd64.AppImage` | `sudo apt install ./<файл>.deb`, `sudo dnf install ./<файл>.rpm` |

Установщики — в разделе релизов `https://github.com/vvnovg/glafira-load-releases/releases`.
Пробелы в именах файлов заменены на `_` при публикации; точные имена смотрите в релизе.
Исполняемый файл приложения — `glafira-load-testing` (`glafira-load-testing.exe` на Windows,
`/usr/bin/glafira-load-testing` на Linux). Подписание установщиков и обновление изнутри приложения появятся на этапе
L7; до этого новые версии раздаются теми же средствами, что и установка.

Приложению нужен системный `git` в `PATH`: им читается версия клонов сервисов (`git describe`, `git rev-parse`).
Без `git` всё работает, но версия сервисов записывается как `unversioned`.

## Агент нагрузки
CLI `glafira-load` — один статический бинарник на платформу, публикуется в релизе рядом с установщиками:
`glafira-load-linux-x86_64`, `glafira-load-linux-aarch64` (musl, без зависимостей от glibc), `glafira-load-macos-aarch64`,
`glafira-load-macos-x86_64`, `glafira-load-windows-x86_64.exe`; рядом — файлы `.sha256`. Версия агента должна совпадать с
версией приложения в мажорной и минорной части (`0.2`): иначе прогон не стартует с `AGENT_VERSION`. Команду скачивания
для своей платформы показывает экран «О программе».

Установка (Linux; на macOS — файл `glafira-load-macos-<arch>` и `shasum -a 256 -c` вместо `sha256sum -c`):
```bash
V=0.2.0
sudo curl -L "https://github.com/vvnovg/glafira-load-releases/releases/download/v$V/glafira-load-linux-x86_64" -o /usr/local/bin/glafira-load
curl -L "https://github.com/vvnovg/glafira-load-releases/releases/download/v$V/glafira-load-linux-x86_64.sha256" | sed 's#  .*#  /usr/local/bin/glafira-load#' | sha256sum -c -
sudo chmod +x /usr/local/bin/glafira-load
glafira-load --version
```
На macOS бинарник не подписан: если он скачан браузером, снимите карантин —
`xattr -d com.apple.quarantine /usr/local/bin/glafira-load` (`curl` атрибут карантина не ставит). Windows:
`curl.exe -L "https://github.com/vvnovg/glafira-load-releases/releases/download/v0.2.0/glafira-load-windows-x86_64.exe" -o C:\glafira-load\glafira-load.exe`.

Агент слушает TCP-порт (в примерах `7420`) и принимает запросы контроллера с заголовком
`Authorization: Bearer <токен>`; запрос с другим токеном получает 401. TLS у агента нет — порт открывают только
внутри сети стенда или ставят агента за обратный прокси с TLS. Параметры:

| Параметр | Смысл |
|---|---|
| `--listen <адрес:порт>` | где слушать, например `0.0.0.0:7420` |
| `--token <t>` / `--token-file <путь>` / `GLAFIRA_LOAD_AGENT_TOKEN` | токен, в этом порядке; без него агент не стартует. Файл токена — только для пользователя службы (`chmod 600`); `--token` виден в списке процессов, для служб используйте файл или переменную |
| `--work <папка>` | бандлы и файлы прогонов; по умолчанию `glafira-load-agent` во временной папке системы — для службы задайте постоянную папку |
| `--keep-runs <N>` | сколько последних прогонов хранить в `--work` (по умолчанию 10); контроллер достраивает незавершённые прогоны из этих файлов |
| `--drain-ms <мс>` | сколько идущим итерациям дорабатывать после остановки (по умолчанию 5000) |
| `GLAFIRA_LOAD_SECRET_<ИМЯ>` | секреты стенда, на которые сьют ссылается как `{{secret ИМЯ}}`; приложение их не хранит и не передаёт |
| `RUST_LOG` | уровень журнала, по умолчанию `info` |

Сетевые соединения агента: входящие от контроллера; исходящие — к сервисам стенда (HTTP, Kafka, Schema Registry) по
адресам из манифеста проекта. Журнал агента — stderr (журнал службы) и `agent.log` в папке прогона внутри `--work`;
контроллер забирает `agent.log` вместе с остальными файлами прогона. Агент завершается по `Ctrl+C` (SIGINT): сначала
останавливает текущий прогон с доработкой `--drain-ms`, затем сервер. Другие сигналы завершают процесс сразу — идущий
прогон тогда обрывается, и контроллер помечает его `incomplete`.

### systemd (Linux)
`/etc/glafira-load/agent.env` (владелец `root`, права `600`; systemd читает его до смены пользователя):
```
GLAFIRA_LOAD_AGENT_TOKEN=замените-на-случайную-строку
GLAFIRA_LOAD_SECRET_API_TOKEN=значение-секрета-стенда
```
`/etc/systemd/system/glafira-load-agent.service`:
```ini
[Unit]
Description=Glafira for Load Testing agent
After=network-online.target
Wants=network-online.target

[Service]
User=glafira-load
Group=glafira-load
EnvironmentFile=/etc/glafira-load/agent.env
ExecStart=/usr/local/bin/glafira-load agent --listen 0.0.0.0:7420 --work /var/lib/glafira-load/agent --keep-runs 10 --drain-ms 5000
# мягкая остановка: SIGINT — как Ctrl+C, прогон дорабатывает --drain-ms
KillSignal=SIGINT
TimeoutStopSec=30
Restart=on-failure
RestartSec=5
LimitNOFILE=65536
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=/var/lib/glafira-load
PrivateTmp=true

[Install]
WantedBy=multi-user.target
```
```bash
sudo useradd --system --home /var/lib/glafira-load --create-home --shell /usr/sbin/nologin glafira-load
sudo install -d -o glafira-load -g glafira-load /var/lib/glafira-load/agent
sudo install -d -m 700 /etc/glafira-load && sudo touch /etc/glafira-load/agent.env && sudo chmod 600 /etc/glafira-load/agent.env
sudo systemctl daemon-reload && sudo systemctl enable --now glafira-load-agent
sudo systemctl status glafira-load-agent
journalctl -u glafira-load-agent -f
```
`LimitNOFILE` поднят: при `max_in_flight: 1000` агент держит до тысячи соединений плюс пул keep-alive.

### launchd (macOS)
`/Library/LaunchDaemons/io.github.vvnovg.glafira.load.agent.plist`:
```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>io.github.vvnovg.glafira.load.agent</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/local/bin/glafira-load</string>
    <string>agent</string>
    <string>--listen</string><string>0.0.0.0:7420</string>
    <string>--token-file</string><string>/usr/local/etc/glafira-load/token</string>
    <string>--work</string><string>/usr/local/var/glafira-load/agent</string>
    <string>--keep-runs</string><string>10</string>
    <string>--drain-ms</string><string>5000</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>GLAFIRA_LOAD_SECRET_API_TOKEN</key><string>значение-секрета-стенда</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>/usr/local/var/log/glafira-load-agent.log</string>
  <key>StandardErrorPath</key><string>/usr/local/var/log/glafira-load-agent.log</string>
</dict>
</plist>
```
```bash
sudo install -d -m 700 /usr/local/etc/glafira-load && sudo sh -c 'openssl rand -hex 32 > /usr/local/etc/glafira-load/token' && sudo chmod 600 /usr/local/etc/glafira-load/token
sudo install -d /usr/local/var/glafira-load/agent /usr/local/var/log
sudo chmod 600 /Library/LaunchDaemons/io.github.vvnovg.glafira.load.agent.plist   # в plist секреты стенда
sudo launchctl bootstrap system /Library/LaunchDaemons/io.github.vvnovg.glafira.load.agent.plist
sudo launchctl print system/io.github.vvnovg.glafira.load.agent | head
```
Токен для приложения — содержимое `/usr/local/etc/glafira-load/token`. Остановить:
`sudo launchctl bootout system/io.github.vvnovg.glafira.load.agent` (launchd шлёт SIGTERM — останавливайте службу между
прогонами).

### Служба Windows
`sc create` запускает только программы, которые умеют говорить с диспетчером служб; `glafira-load.exe` — обычная
консольная программа, поэтому служба создаётся через `cmd /c` с перенаправлением, а токен и секреты задаются
переменными окружения машины (PowerShell от администратора):
```powershell
New-Item -ItemType Directory -Force C:\glafira-load\agent | Out-Null
[Environment]::SetEnvironmentVariable("GLAFIRA_LOAD_AGENT_TOKEN", "замените-на-случайную-строку", "Machine")
[Environment]::SetEnvironmentVariable("GLAFIRA_LOAD_SECRET_API_TOKEN", "значение-секрета-стенда", "Machine")
sc.exe create GlafiraLoadAgent binPath= "cmd.exe /c \"C:\glafira-load\glafira-load.exe agent --listen 0.0.0.0:7420 --work C:\glafira-load\agent --keep-runs 10 --drain-ms 5000 >> C:\glafira-load\agent.log 2>&1\"" start= auto DisplayName= "Glafira for Load Testing agent"
sc.exe start GlafiraLoadAgent
netsh advfirewall firewall add rule name="Glafira load agent" dir=in action=allow protocol=TCP localport=7420
```
Так остановка службы завершает `cmd.exe`, а не агента, и идущий прогон не получает мягкой остановки; для продуктивных
машин нагрузки надёжнее планировщик заданий (`schtasks /create /sc onstart /ru SYSTEM …`) или обёртка-служба
(например, `nssm`), которая перезапускает процесс и передаёт ему завершение. Переменные уровня машины видны всем
процессам: секреты стенда на Windows лучше хранить в учётной записи службы (`/ru`) с ограниченным доступом.

## Политика
Приложение читает политику при запуске. Пользователь не может её изменить.

| ОС | Где |
|---|---|
| macOS | профиль конфигурации MDM, домен `io.github.vvnovg.glafira.load` (файл попадает в `/Library/Managed Preferences/io.github.vvnovg.glafira.load.plist`) — пример `admin/policy.mobileconfig` |
| Windows | `HKLM\SOFTWARE\Policies\Glafira\LoadTesting` (GPO, Intune) — пример `admin/policy.reg` |
| Linux | `/etc/io.github.vvnovg.glafira.load/policy.json` — пример `admin/policy.json` |

| Ключ | Тип | По умолчанию | Смысл |
|---|---|---|---|
| `updatesEnabled` | bool (`dword` 0/1 в реестре) | `true` | `false` — приложение не обращается к серверу обновлений |
| `updateUrl` | строка, только `https://` | публичный `latest.json` | зеркало с одобренными версиями (формат — `latest.json` Tauri). Адрес `http://` считается ошибкой политики: обновления выключаются |
| `language` | строка: `en` или `ru` (точно в этом написании) | язык ОС или выбор пользователя | язык интерфейса для всех пользователей; пользователь его не меняет. Неизвестный код — ошибка политики (показывается в «О программе»), язык не задаётся, обновления не выключаются |

Если политика есть, но не читается или содержит недопустимое значение `updatesEnabled` или `updateUrl`, обновления
выключаются, а в «О программе» показывается ошибка политики. Неизвестные ключи игнорируются с предупреждением в логе.

### Проверка политики вручную (macOS)
`defaults write` в `/Library/Managed Preferences` на современной macOS не работает (туда пишет только MDM).
Для проверки соберите plist и скопируйте его от имени администратора:

```bash
rm -f /tmp/policy.plist && plutil -create xml1 /tmp/policy.plist
plutil -insert updatesEnabled -bool NO /tmp/policy.plist
sudo mkdir -p "/Library/Managed Preferences"
sudo cp /tmp/policy.plist "/Library/Managed Preferences/io.github.vvnovg.glafira.load.plist"
sudo chmod 644 "/Library/Managed Preferences/io.github.vvnovg.glafira.load.plist"
plutil -p "/Library/Managed Preferences/io.github.vvnovg.glafira.load.plist"
```
Удалить: `sudo rm "/Library/Managed Preferences/io.github.vvnovg.glafira.load.plist"`.

## Сеть
Приложение подключается только к серверу обновлений, если обновления не выключены: `github.com` и хосты, куда GitHub
перенаправляет загрузку ассетов релизов (`release-assets.githubusercontent.com`, `objects.githubusercontent.com`), —
или к хосту из `updateUrl`. Сборки 0.1.0 сделаны без ключа обновлений и к серверу обновлений не обращаются вовсе.

С версии 0.2.0 приложение подключается к агентам нагрузки по адресам из манифеста проекта
(`environments[].agents[].url`, HTTP внутри сети стенда или HTTPS за обратным прокси), а смок и прогон на стенде без
агентов идут внутри приложения и подключаются к сервисам стенда (HTTP, Kafka, Schema Registry) по адресам из манифеста.
Других соединений нет: нагрузку на стенд с агентами создают агенты, а не приложение. `glafira-load run` без `--agents`
подключается к сервисам стенда сам.

Проекты, клоны сервисов и контракты читаются только с локального диска; `$ref` на внешние URL не загружаются. Телеметрии
и отчётов о сбоях нет. Ссылки в «О программе» открываются в браузере только по нажатию пользователя.

## Где хранятся данные
| | macOS | Windows | Linux |
|---|---|---|---|
| Настройки | `~/Library/Application Support/io.github.vvnovg.glafira.load` | `%APPDATA%\io.github.vvnovg.glafira.load` | `~/.local/share/io.github.vvnovg.glafira.load` |
| Логи | `~/Library/Logs/io.github.vvnovg.glafira.load` | `%LOCALAPPDATA%\io.github.vvnovg.glafira.load\logs` | `~/.local/share/io.github.vvnovg.glafira.load/logs` |

Пути Windows и Linux — по правилам Tauri (`app_data_dir`, `app_log_dir`); точные пути показывает «О программе».

- В каталоге данных — `settings.json`: язык интерфейса, список открытых проектов и последний активный проект.
- Проекты НТ (манифест, реестр операций, цифры нагрузки ПРОМ, профиль) лежат там, где их создал пользователь, — обычно
  в git-репозиториях команды. Приложение не копирует их в каталог данных.
- Клоны сервисов приложение только читает.
- Токены агентов — в системном хранилище секретов (Keychain, Credential Manager, Secret Service), сервис `glafira-load`,
  ключ `<папка проекта>/<стенд>/<агент>`; в файлы проекта они не попадают. После переноса папки проекта токены вводятся
  заново. Секреты стендов приложение не хранит: они задаются на агентах переменными `GLAFIRA_LOAD_SECRET_<ИМЯ>` (для
  смока и стенда без агентов — в окружении самого приложения).
- Результаты прогонов — `runs/<дата>/` в папке проекта (в `.gitignore`).
