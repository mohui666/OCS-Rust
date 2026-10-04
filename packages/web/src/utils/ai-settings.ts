import { invoke, isNative } from './native';

export function aiPageParams() {
  return new URLSearchParams(location.hash.split('?')[1] || location.search);
}

export async function aiSettingsRequest(action: string, config?: Record<string, unknown>): Promise<any> {
  if (isNative) return invoke('bridge_control', { action, config: config || null });
  const token = aiPageParams().get('token');
  if (!token) throw new Error('请从 OCS 脚本的“答题设置”打开此面板。');
  const response = await fetch('/ai-control', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'auth-token': token },
    body: JSON.stringify({ action, config })
  });
  const value = await response.json();
  if (!response.ok) throw new Error(value.error || '无法连接 OCS，请确认桌面程序正在运行。');
  return value;
}
