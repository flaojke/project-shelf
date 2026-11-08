import { useCallback, useEffect, useRef, useState } from 'react';
import { api, isDemo } from '../lib/api';
import type { ProjectDiagnostics, ProjectStatus } from '../lib/types';
import { Icon } from './Icons';

type Check = ProjectDiagnostics['checks'][number];
type Props = {
  projectId: string;
  status: ProjectStatus;
  busyAction?: string;
  onRestart: () => void;
  onStop: () => void;
};
const checkLabels: Record<Check['state'], string> = {
  checking: 'Проверяем', unknown: 'Не проверено', ready: 'Готово', error: 'Ошибка', notApplicable: 'Не требуется',
};
const emptyChecks: { id: Check['id']; label: string }[] = [
  { id: 'dependencies', label: 'Компоненты запуска' },
  { id: 'process', label: 'Процесс или сервис' },
  { id: 'http', label: 'HTTP-ответ' },
  { id: 'port', label: 'Локальный порт' },
];

export function ReadinessPanel({ projectId, status, busyAction, onRestart, onStop }: Props) {
  const [report, setReport] = useState<ProjectDiagnostics | null>(null);
  const [loading, setLoading] = useState(false);
  const [failure, setFailure] = useState('');
  const request = useRef(0);
  const inFlight = useRef(false);
  const active = useRef(false);
  const refreshQueued = useRef(false);
  const latestRefresh = useRef<() => Promise<void>>(async () => {});
  const statusKey = JSON.stringify([status.phase, status.owned, status.pid, status.serial, busyAction]);
  const previousStatusKey = useRef(statusKey);

  const refresh = useCallback(async () => {
    if (!active.current) return;
    if (inFlight.current) { refreshQueued.current = true; return; }
    inFlight.current = true;
    const token = ++request.current;
    setLoading(true);
    try {
      const next = await api.projectDiagnostics(projectId);
      if (!active.current || token !== request.current) return;
      if (!next || next.id !== projectId || !Array.isArray(next.checks)) throw new Error('Диспетчер не вернул полные данные диагностики.');
      setReport(next); setFailure('');
    } catch (error) {
      if (!active.current || token !== request.current) return;
      setReport(null); setFailure(error instanceof Error ? error.message : String(error));
    } finally {
      // The RPC owns this flag until it actually settles. StrictMode cleanup
      // invalidates its result, but must not pretend the RPC has finished.
      inFlight.current = false;
      if (active.current && token === request.current) setLoading(false);
      if (active.current && refreshQueued.current) {
        refreshQueued.current = false;
        void latestRefresh.current();
      }
    }
  }, [projectId]);
  latestRefresh.current = refresh;

  useEffect(() => {
    active.current = true;
    setReport(null); setFailure('');
    void refresh();
    const timer = window.setInterval(() => void refresh(), 4000);
    return () => { active.current = false; ++request.current; refreshQueued.current = false; window.clearInterval(timer); };
  }, [refresh]);
  useEffect(() => {
    if (previousStatusKey.current === statusKey) return;
    previousStatusKey.current = statusKey;
    void refresh();
  }, [refresh, statusKey]);

  const checks: Check[] = emptyChecks.map(base => report?.checks.find(item => item.id === base.id) || {
    ...base, state: loading ? 'checking' : 'unknown', message: loading ? 'Получаем текущие данные проверки.' : 'Результат проверки пока недоступен.',
  });
  const owned = status.owned;
  const canRestart = report?.canRestart === true && owned && !busyAction && !failure;
  const canStop = report?.canStop === true && owned && !busyAction && !failure;
  const pendingReason = loading && !report ? 'Проверяем возможности управления.' : failure ? 'Сначала обновите проверку готовности.' : !owned ? 'Доступно только для запуска, которым управляет эта сессия Project Shelf.' : 'Возможность действия пока не подтверждена.';
  const restartReason = busyAction ? 'Дождитесь завершения текущего действия.' : canRestart ? 'Безопасная остановка и повторный запуск этого проекта.' : report?.restartReason || pendingReason;
  const stopReason = busyAction ? 'Дождитесь завершения текущего действия.' : canStop ? 'Завершится только подтверждённый управляемый запуск.' : report?.stopReason || pendingReason;
  const date = report?.checkedAt ? new Date(report.checkedAt) : null;
  const checkedAt = date && Number.isFinite(date.getTime()) ? date.toLocaleTimeString('ru-RU', { hour: '2-digit', minute: '2-digit', second: '2-digit' }) : '';

  return <section className="readiness-panel" aria-label="Проверка готовности проекта">
    <div className="readiness-heading"><div><h2>Проверка запуска</h2><p>{isDemo ? 'Демо: результаты проверок искусственные.' : checkedAt ? `Последняя проверка · ${checkedAt}` : 'Показываем только полученные результаты.'}</p></div><button className="icon-button readiness-refresh" aria-label="Обновить проверку готовности" title="Обновить проверку готовности" disabled={loading} onClick={() => void refresh()}><Icon name="refresh" size={16} className={loading ? 'spin' : ''}/></button></div>
    <div className="readiness-checks" role="list" aria-label="Результаты проверки запуска">
      {checks.map(check => {
        const value = check.id === 'http' && typeof check.httpStatus === 'number' ? `HTTP ${check.httpStatus}` : check.value;
        return <div className={`readiness-check check-${check.state}`} key={check.id} role="listitem" data-check-id={check.id} data-state={check.state}>
          <div className="check-topline"><span className="check-icon"><Icon name={check.state === 'checking' ? 'refresh' : check.state === 'ready' ? 'check' : check.state === 'error' ? 'alert' : 'info'} size={15} className={check.state === 'checking' ? 'spin' : ''}/></span><span className="check-state">{checkLabels[check.state]}</span></div>
          <h3>{check.label}</h3>{value && <div className="check-value">{value}</div>}<p>{check.message}</p>
        </div>;
      })}
    </div>
    {report?.technologies?.length ? <div className="detected-technologies"><span>Определено при проверке</span><div>{report.technologies.map((technology, index) => <span key={`${technology}-${index}`}>{technology}</span>)}</div></div> : null}
    {failure && <div className="inline-note error-note readiness-error" role="alert"><Icon name="alert" size={16}/><span>Не удалось обновить диагностику: {failure}</span></div>}
    <div className="readiness-controls"><div className="readiness-control"><button className="button button-secondary" disabled={!canRestart} onClick={onRestart} title={restartReason}><Icon name="refresh" size={15} className={busyAction === 'restart' ? 'spin' : ''}/>{busyAction === 'restart' ? 'Перезапускаем…' : 'Перезапустить'}</button><p>{restartReason}</p></div><div className="readiness-control"><button className="button button-secondary" disabled={!canStop} onClick={onStop} title={stopReason}><Icon name={busyAction === 'stop' ? 'refresh' : 'stop'} size={15} className={busyAction === 'stop' ? 'spin' : ''}/>{busyAction === 'stop' ? 'Останавливаем…' : 'Остановить'}</button><p>{stopReason}</p></div></div>
  </section>;
}
