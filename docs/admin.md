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
- Секретов в 0.1.0 нет: у стендов в манифесте только адреса без паролей. Пароли стендов появятся вместе с раннером
  (этап L2) и будут храниться в системном хранилище секретов (Keychain, Credential Manager, Secret Service), а не в
  файлах проекта.
