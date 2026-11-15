<div align="center">

# GoVPN Lab

**IP-туннель на Rust поверх TLS 1.3**

Linux · IPv4 и IPv6 · Пакетная обработка TUN

[Скачать](https://github.com/doctorBax/govpn-lab/releases/latest) · [Запуск](#запуск) · [Замеры](#замеры) · [История версий](CHANGELOG.md)

</div>

---

GoVPN Lab передаёт IP-пакеты между двумя Linux-узлами. Клиент проверяет сертификат сервера через TLS и аутентифицируется общим секретом. Версия на Rust совместима с Python-клиентом и сервером из v0.1.0.

```text
Linux TUN  ←→  Клиент  ═══════ TLS 1.3 ═══════  Сервер  ←→  Linux TUN
```

## Запуск

Скачайте и распакуйте архив [последнего релиза](https://github.com/doctorBax/govpn-lab/releases/latest) для Linux x86_64 с glibc 2.34 или новее. Для сборки из исходников выполните `cargo build --release --locked`. При локальной сборке исполняемый файл появится в `target/release/govpn-lab`.

На сервере:

```bash
./govpn-lab server --host 0.0.0.0 --port 9443 --device tun \
  --cert /path/to/fullchain.pem --key /path/to/privkey.pem \
  --secret-file /path/to/session.bin --tun-name labtun0
```

На клиенте:

```bash
./govpn-lab client --host vpn.example.com --port 9443 \
  --server-name vpn.example.com --secret-file /path/to/session.bin \
  --tun-name labtun0
```

Оба узла используют один файл с общим секретом длиной не менее 32 случайных байт. Если сертификат подписан вашим центром сертификации, передайте клиенту его сертификат через `--ca`. Режим TUN требует `/dev/net/tun` и прав на создание интерфейса. Адреса интерфейсов, маршруты и NAT настраиваются средствами Linux.

Для проверки без маршрутов используйте сервер в режиме `--device echo` и команду `govpn-lab probe` с теми же параметрами TLS и секрета. По умолчанию сервер слушает loopback в echo-режиме.

## Замеры

В лабораторном тесте на Debian 12 один и тот же Python-клиент отправлял по 131 072 пакета через TLS на loopback. CPU измерялся у процесса сервера. В режиме echo сервер возвращает каждый пакет, а клиент проверяет его содержимое и порядок.

| Размер пакета | v0.1.0 · Python | v0.2.0 · Rust | CPU Python → Rust |
| --- | ---: | ---: | ---: |
| 64 Б | 31 424 пак./с | 116 798 пак./с | 4,09 → 0,31 с |
| 1280 Б | 25 174 пак./с | 67 821 пак./с | 5,16 → 0,88 с |

Скорость на внешнем маршруте и LTE в этом замере не измерялась. В режиме TUN Rust переиспользует буферы и отправляет пачкой уже доступные пакеты. Формат кадров остался прежним.

## Проверка

```bash
cargo test --locked
python3 -m pip install -r requirements-dev.txt
python3 -m scripts.verify target/release/govpn-lab --bench
sudo bash scripts/tun-check.sh target/release/govpn-lab
```

У скрипта проверки TUN есть зависимости от Python 3, `ip`, `ping`, `unshare` и `nsenter`. Он создаёт два временных сетевых пространства имён и проверяет обмен IPv4 и IPv6 между ними.
