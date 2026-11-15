#!/usr/bin/env bash
set -euo pipefail

host=${1:-}
upgrade=false
if [[ $host == --upgrade ]]; then upgrade=true; fi
if [[ $EUID != 0 ]] || { ! $upgrade && [[ ! $host =~ ^[A-Za-z0-9][A-Za-z0-9.-]*$ ]]; }; then
  echo 'Usage: sudo bash install-server.sh <public IPv4 or DNS name | --upgrade>' >&2
  exit 2
fi
if [[ $(uname -m) != x86_64 || ! -c /dev/net/tun || ! -d /run/systemd/system ]]; then
  echo 'Requires x86_64 Linux with systemd and /dev/net/tun (for example Ubuntu 22.04).' >&2
  exit 1
fi
for command in curl tar sha256sum openssl ip iptables sysctl systemctl; do
  command -v "$command" >/dev/null || { echo "Missing dependency: $command" >&2; exit 1; }
done
if ! command -v conntrack >/dev/null; then
  echo 'Install conntrack first (Ubuntu: apt-get install conntrack).' >&2
  exit 1
fi
dir=/etc/sslim
files=(/usr/local/bin/sslim /usr/local/bin/sslim-client "$dir/network" /etc/systemd/system/sslim.service)
if $upgrade; then
  for path in "${files[@]}" "$dir"/{hostname,ca.pem,server.pem,server.key,clients.txt}; do
    [[ -f $path && ! -L $path ]] || { echo "Standard installation file missing: $path" >&2; exit 1; }
  done
elif [[ -e $dir || -e /usr/local/bin/sslim || -e /usr/local/bin/sslim-client || -e /etc/systemd/system/sslim.service ]] || ip link show sslim0 >/dev/null 2>&1; then
  echo 'An SSLim installation or interface already exists. Inspect it before reinstalling.' >&2
  exit 1
fi
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT
assets=https://github.com/doctorBax/SSLim/releases/latest/download
curl -fsSL "$assets/sslim-linux-x86_64.tar.gz" -o "$tmp/sslim-linux-x86_64.tar.gz"
curl -fsSL "$assets/SHA256SUMS" -o "$tmp/SHA256SUMS"
(cd "$tmp"; sha256sum --ignore-missing --check SHA256SUMS)
tar -xzf "$tmp/sslim-linux-x86_64.tar.gz" -C "$tmp" sslim
"$tmp/sslim" --version
"$tmp/sslim" server --help | grep -q -- '--multi-tun'

umask 077
if $upgrade; then
  backup=$(mktemp -d "$dir/upgrade-backup.XXXXXXXX")
  cp -p "${files[0]}" "$backup/sslim"
  cp -p "${files[1]}" "$backup/sslim-client"
  cp -p "${files[2]}" "$backup/network"
  cp -p "${files[3]}" "$backup/sslim.service"
  systemctl stop sslim.service
  rollback() {
    code=$?
    if (( code != 0 )); then
      systemctl stop sslim.service || true
      cp -p "$backup/sslim" "${files[0]}"
      cp -p "$backup/sslim-client" "${files[1]}"
      cp -p "$backup/network" "${files[2]}"
      cp -p "$backup/sslim.service" "${files[3]}"
      systemctl daemon-reload
      systemctl start sslim.service
    fi
    rm -rf -- "$tmp"
    exit "$code"
  }
  trap rollback EXIT
else
install -d -m 0700 "$dir"
printf '%s\n' "$host" > "$dir/hostname"
san=DNS:$host
if [[ $host =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]]; then san=IP:$host; fi
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 365 \
  -subj /CN=SSLim-CA -keyout "$tmp/ca.key" -out "$dir/ca.pem" \
  -addext 'basicConstraints=critical,CA:TRUE' -addext 'keyUsage=critical,keyCertSign,cRLSign'
openssl req -new -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
  -subj "/CN=$host" -keyout "$dir/server.key" -out "$tmp/server.csr"
printf 'subjectAltName=%s\nextendedKeyUsage=serverAuth\n' "$san" > "$tmp/extensions"
openssl x509 -req -in "$tmp/server.csr" -CA "$dir/ca.pem" -CAkey "$tmp/ca.key" \
  -CAserial "$tmp/ca.srl" -CAcreateserial -out "$dir/server.pem" -days 365 -extfile "$tmp/extensions"
fi
install -m 0755 "$tmp/sslim" /usr/local/bin/sslim
cat > /usr/local/bin/sslim-client <<'EOF'
#!/bin/sh
set -eu
dir=/etc/sslim
case "${1:-}" in
  add|link|rotate)
       [ "$#" -eq 2 ] || { echo "Usage: sslim-client $1 NAME" >&2; exit 2; }
       action=$1
       set -- --label "$2" --host "$(cat "$dir/hostname")" --ca-file "$dir/ca.pem" --secrets-file "$dir/clients.txt"
       case "$action" in
         link) set -- "$@" --show ;;
         rotate) set -- "$@" --rotate ;;
       esac
       exec sslim clients add "$@" ;;
  list) exec sslim clients list --secrets-file "$dir/clients.txt" ;;
  remove) [ "$#" -eq 2 ] || exit 2
          exec sslim clients remove "$2" --secrets-file "$dir/clients.txt" ;;
  status) exec systemctl --no-pager status sslim.service ;;
  *) echo 'Commands: sslim-client add NAME | list | link NAME | rotate NAME | remove NAME | status' ;;
esac
EOF
chmod 0755 /usr/local/bin/sslim-client
if ! $upgrade; then /usr/local/bin/sslim-client add first > "$tmp/profile"; fi

cat > "$dir/network" <<'EOF'
#!/bin/bash
set -euo pipefail
if [[ $1 == up ]]; then
  # Addresses restart at the beginning of the pool; discard previous NAT state.
  entries=$(conntrack -L -f ipv4 -s 10.253.0.0/16)
  if [[ -n $entries ]]; then
    conntrack -D -f ipv4 -s 10.253.0.0/16
  fi
  ip tuntap add dev sslim0 mode tun
  ip addr add 10.253.0.1 peer 10.253.0.2 dev sslim0
  ip link set sslim0 mtu 1280 up
  sysctl -w net.ipv4.ip_forward=1
  iptables -w -I FORWARD -i sslim0 -s 10.253.0.2/32 -m comment --comment sslim -j ACCEPT
  iptables -w -I FORWARD -o sslim0 -d 10.253.0.2/32 -m conntrack --ctstate ESTABLISHED,RELATED -m comment --comment sslim -j ACCEPT
  iptables -w -t nat -A POSTROUTING -s 10.253.0.2/32 ! -o sslim0 -m comment --comment sslim -j MASQUERADE
  iptables -w -I INPUT -i sslimd+ -j DROP
  iptables -w -I FORWARD -i sslim+ -o sslim+ -j DROP
  iptables -w -I FORWARD -i sslimd+ ! -o sslim+ -s 10.253.0.0/16 -j ACCEPT
  iptables -w -I FORWARD ! -i sslim+ -o sslimd+ -d 10.253.0.0/16 -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
  iptables -w -t nat -A POSTROUTING -s 10.253.0.0/16 ! -o sslim+ -m comment --comment sslim-multi -j MASQUERADE
else
  iptables -w -t nat -D POSTROUTING -s 10.253.0.0/16 ! -o sslim+ -m comment --comment sslim-multi -j MASQUERADE || true
  iptables -w -D FORWARD ! -i sslim+ -o sslimd+ -d 10.253.0.0/16 -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT || true
  iptables -w -D FORWARD -i sslimd+ ! -o sslim+ -s 10.253.0.0/16 -j ACCEPT || true
  iptables -w -D FORWARD -i sslim+ -o sslim+ -j DROP || true
  iptables -w -D INPUT -i sslimd+ -j DROP || true
  iptables -w -t nat -D POSTROUTING -s 10.253.0.2/32 ! -o sslim0 -m comment --comment sslim -j MASQUERADE || true
  iptables -w -D FORWARD -o sslim0 -d 10.253.0.2/32 -m conntrack --ctstate ESTABLISHED,RELATED -m comment --comment sslim -j ACCEPT || true
  iptables -w -D FORWARD -i sslim0 -s 10.253.0.2/32 -m comment --comment sslim -j ACCEPT || true
  ip link delete sslim0 || true
fi
EOF
chmod 0700 "$dir/network"
cat > /etc/systemd/system/sslim.service <<'EOF'
[Unit]
Description=SSLim packet tunnel
After=network-online.target
Wants=network-online.target

[Service]
ExecStartPre=/etc/sslim/network up
ExecStart=/usr/local/bin/sslim server --host 0.0.0.0 --device tun --tun-name sslim0 --multi-tun --cert /etc/sslim/server.pem --key /etc/sslim/server.key --secrets-file /etc/sslim/clients.txt
ExecStopPost=/etc/sslim/network down
Restart=on-failure
RestartSec=3

[Install]
WantedBy=multi-user.target
EOF
chmod 0644 /etc/systemd/system/sslim.service
systemctl daemon-reload
systemctl enable --now sslim.service
if ! timeout 15 bash -c 'until (echo > /dev/tcp/127.0.0.1/9443) 2>/dev/null; do sleep 0.25; done'; then
  echo 'SSLim did not open TCP 9443.' >&2
  exit 1
fi
systemctl is-active --quiet sslim.service
if $upgrade; then
  echo "SSLim upgraded. Existing links still work. Previous files: $backup"
  exit 0
fi
echo 'Allow inbound TCP port 9443 in your VPS firewall. Copy this private link into Blade:'
cat "$tmp/profile"
