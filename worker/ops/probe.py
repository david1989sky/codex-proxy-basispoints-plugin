#!/usr/bin/env python3
import http.cookiejar
import json
import sys
import urllib.error
import urllib.request
from pathlib import Path
import yaml

origin = "https://cx.subarx.com"
base = "http://127.0.0.1:28080"
jar = http.cookiejar.CookieJar()
client = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))

def request(path, body=None, destination=base, extra=None, method=None):
    headers = {"Origin": origin, "Content-Type": "application/json", **(extra or {})}
    req = urllib.request.Request(destination + path, data=json.dumps(body).encode() if body is not None else None, headers=headers, method=method)
    try:
        with client.open(req, timeout=40) as response:
            return response.status, json.load(response)
    except urllib.error.HTTPError as error:
        return error.code, None

config = yaml.safe_load(Path('/opt/codex-proxy-rs-v3.8.0/deploy/config.yaml').read_text())
admin = config['admin']
status, _ = request('/api/auth/login', {"mode": "admin", "username": admin['default_username'], "password": admin['default_password']})
config = admin = None
print('admin_login_status', status)
if status != 200:
    sys.exit(1)
# 仅在进程内转发 Cookie，不输出或持久化会话内容。
cookie = '; '.join(f'{item.name}={item.value}' for item in jar)
extra = {'Cookie': cookie, 'X-CPR-TwoFA': '1'}
status, data = request('/api/auth/status', extra=extra)
print('session_role', data['data']['session']['role'] if data else 'missing')
status, data = request('/api/admin/proxies?page=1&pageSize=100', extra=extra)
print('proxies', [{'id': p['id'], 'endpoint': p['endpoint'], 'hasAuthentication': p['hasAuthentication'], 'tested': bool((p.get('lastTest') or {}).get('success'))} for p in (data or {}).get('data', {}).get('items', [])])
if '--sidecar' in sys.argv:
    worker = 'http://127.0.0.1:28082'
    status, _ = request('/api/admin/twofa/tasks/missing', destination=worker, extra=extra)
    print('authenticated_task_lookup_status', status)
    assert status == 404
    status, _ = request('/api/admin/twofa/tasks', {'text': 'invalid', 'submissionId': 'probe', 'settings': {'enabled': True, 'concurrencyLimit': None, 'weight': 1, 'groupIds': []}}, destination=worker, extra=extra)
    print('invalid_txt_status', status)
    assert status == 400
    status, _ = request('/api/admin/twofa/tasks/missing', destination=origin, extra=extra)
    print('public_authenticated_task_lookup_status', status)
    assert status == 404
status, _ = request('/api/auth/logout', {}, extra=extra)
print('probe_session_logout_status', status)
