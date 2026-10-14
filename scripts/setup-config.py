#!/usr/bin/env python3
"""Create a private credentials file without exposing passwords in arguments."""
import argparse
import getpass
import json
import os
from pathlib import Path
from urllib.parse import urlsplit


def create_config(path, url, username, password):
    parsed = urlsplit(url)
    if (parsed.scheme not in ('http', 'https') or not parsed.hostname
            or parsed.username or parsed.password or parsed.path not in ('', '/')
            or parsed.query or parsed.fragment):
        raise ValueError('URL must be an HTTP(S) origin without embedded credentials')
    if not username or not password:
        raise ValueError('Username and password must not be empty')
    content = (f'url = {json.dumps(url)}\nusername = {json.dumps(username)}\n'
               f'password = {json.dumps(password)}\ntimeout_seconds = 15\n'
               'protected_ports = []\n')
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'w') as file:
        file.write(content)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, default=Path.home() / '.netgear-poe.toml')
    parser.add_argument('--url', default='http://192.168.1.250')
    parser.add_argument('--username', default='admin')
    args = parser.parse_args()
    if args.config.exists() or args.config.is_symlink():
        parser.exit(1, 'Credentials file already exists; left untouched.\n')
    if args.url.startswith('http:'):
        print('Warning: HTTP sends login/session traffic unencrypted on your LAN.')
    password = getpass.getpass('Switch password (not stored in command history): ')
    try:
        create_config(args.config, args.url, args.username, password)
    except (ValueError, OSError):
        parser.exit(1, 'Could not create credentials file; check URL, permissions and path.\n')
    print(f'Created {args.config} with mode 0600. Set protected_ports before cycling.')


if __name__ == '__main__':
    main()
