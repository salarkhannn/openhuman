import { useCallback, useState } from 'react';
import { LuThumbsDown, LuThumbsUp } from 'react-icons/lu';

import { useT } from '../../../lib/i18n/I18nContext';
import { callCoreRpc } from '../../../services/coreRpcClient';

/** Langfuse score name every thumbs rating is filed under. */
const FEEDBACK_SCORE_NAME = 'user-feedback';

/** Langfuse `NUMERIC` score values for the two thumbs. */
const GOOD = 1;
const BAD = 0;

type Rating = typeof GOOD | typeof BAD;

interface MessageFeedbackProps {
  /**
   * Langfuse trace this turn exported, read from the message's own
   * `extraMetadata.traceId`.
   *
   * The core stamps it when it persists the reply
   * (`web_chat::reply_persistence`), which is the only reason the renderer can
   * name the right trace: the id is `<trace session id>:<request id>`, and the
   * session half is not something the frontend knows. A turn that exported no
   * trace carries no key, and the caller renders nothing rather than
   * submitting a score that Langfuse would silently orphan.
   */
  traceId: string;
}

/**
 * Thumbs up/down on an assistant reply, submitting a Langfuse score against
 * the turn's trace (#4496).
 *
 * Deliberately non-blocking: a rating is a side remark about a finished turn,
 * so a failure warns in the console and releases the button rather than
 * raising anything over the chat. It does **not** silently keep the button
 * looking "submitted" — the core returns `ok: false` when the push was
 * refused, and showing a confirmed rating for a score that never landed is the
 * exact failure this feature exists to avoid.
 */
export function MessageFeedback({ traceId }: MessageFeedbackProps) {
  const { t } = useT();
  const [submitted, setSubmitted] = useState<Rating | null>(null);
  const [pending, setPending] = useState<Rating | null>(null);

  const submit = useCallback(
    async (value: Rating) => {
      // Already recorded, or a click landed while the previous one is still in
      // flight: either way a second identical `score-create` would just add a
      // duplicate event to the same trace.
      if (submitted === value || pending !== null) return;
      setPending(value);
      try {
        const result = await callCoreRpc<{ ok: boolean; error?: string }>({
          method: 'openhuman.observability_submit_score',
          params: { trace_id: traceId, name: FEEDBACK_SCORE_NAME, value },
        });
        if (result?.ok) {
          setSubmitted(value);
        } else {
          console.warn('[feedback] score was not recorded', result?.error);
        }
      } catch (error) {
        console.warn('[feedback] failed to submit response score', error);
      } finally {
        setPending(null);
      }
    },
    [pending, submitted, traceId]
  );

  const buttonClass = (value: Rating) =>
    [
      'rounded-full p-1 transition-colors disabled:opacity-60',
      submitted === value
        ? 'bg-primary-100 text-primary-600 dark:bg-primary-500/25 dark:text-primary-300'
        : 'text-content-faint hover:bg-surface-muted hover:text-content-secondary',
    ].join(' ');

  return (
    <div className="flex items-center gap-0.5 px-1" data-testid="message-feedback">
      <button
        type="button"
        data-analytics-id="chat-message-feedback-good"
        disabled={pending !== null}
        aria-pressed={submitted === GOOD}
        onClick={() => void submit(GOOD)}
        className={buttonClass(GOOD)}
        title={t('chat.feedback.goodResponse')}
        aria-label={t('chat.feedback.goodResponse')}>
        <LuThumbsUp className="h-3 w-3" aria-hidden />
      </button>
      <button
        type="button"
        data-analytics-id="chat-message-feedback-bad"
        disabled={pending !== null}
        aria-pressed={submitted === BAD}
        onClick={() => void submit(BAD)}
        className={buttonClass(BAD)}
        title={t('chat.feedback.badResponse')}
        aria-label={t('chat.feedback.badResponse')}>
        <LuThumbsDown className="h-3 w-3" aria-hidden />
      </button>
    </div>
  );
}

/**
 * Render {@link MessageFeedback} only for a turn that actually has a trace.
 *
 * Mirrors `MessageCitations`: the metadata bag is untyped, so the guard lives
 * here rather than at every call site.
 */
export function MessageFeedbackRail({ traceId }: { traceId: unknown }) {
  if (typeof traceId !== 'string' || traceId.length === 0) return null;
  return <MessageFeedback traceId={traceId} />;
}
