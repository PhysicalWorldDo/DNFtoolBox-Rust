export class ToolboxError extends Error {
  constructor(detail) {
    super(detail.message || '操作失败，请重试。');
    this.name = 'ToolboxError';
    Object.assign(this, detail);
  }
}

export function createBridge(native = globalThis.__TAURI__) {
  let counter = 0;
  const prefix = `ui-${Date.now().toString(36)}`;
  return {
    available: Boolean(native?.core?.invoke && native?.event?.listen),
    async dispatch(method, params = {}) {
      if (!native?.core?.invoke) throw new ToolboxError({code: 'HOST_UNAVAILABLE', message: '请从 PhysicalWorldToolbox.exe 打开工具箱。', retryable: false});
      const request = {v: 1, id: `${prefix}-${++counter}`, method, params};
      let response;
      try { response = await native.core.invoke('toolbox_dispatch', {request}); }
      catch (error) {
        throw new ToolboxError({code: 'TRANSPORT', message: typeof error === 'string' ? error : error?.message || '与工具箱宿主的连接中断，请重新载入状态。', retryable: true, outcomeUnknown: true});
      }
      if (response?.v !== 1 || response?.id !== request.id || typeof response.ok !== 'boolean') throw new ToolboxError({code: 'PROTOCOL', message: '宿主返回了无法识别的响应，请重新打开工具箱。', retryable: false});
      if (!response.ok) throw new ToolboxError(response.error || {code: 'UNKNOWN', message: '操作未完成。'});
      return response.result;
    },
    async listen(callback) {
      if (!native?.event?.listen) throw new ToolboxError({code: 'HOST_UNAVAILABLE', message: '工具箱事件通道不可用，请重新打开程序。', retryable: true});
      return native.event.listen('toolbox://event', event => callback(event.payload), {target: 'main'});
    }
  };
}
