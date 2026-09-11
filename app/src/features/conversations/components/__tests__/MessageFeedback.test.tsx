import { render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { store } from '../../../../store';
import { MessageFeedback, MessageFeedbackRail } from '../MessageFeedback';

// Issue #4496: thumbs on an assistant reply submit a Langfuse score against
// the turn's own trace. Two properties are load-bearing and each has cost a
// review round: the score must name the trace the core stamped on the message
// (a fabricated id is accepted by Langfuse and then silently orphaned), and a
// refused submission must not leave the UI showing a confirmed rating.

const { mockCallCoreRpc } = vi.hoisted(() => ({ mockCallCoreRpc: vi.fn() }));

vi.mock('../../../../services/coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
}));

function renderFeedback(traceId = 'thread-7:req-42') {
  return render(
    <Provider store={store}>
      <MessageFeedback traceId={traceId} />
    </Provider>
  );
}

describe('MessageFeedback', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockCallCoreRpc.mockResolvedValue({ ok: true });
  });

  it('submits the trace id the core stamped, not a value derived in the UI', async () => {
    renderFeedback('thread-7:req-42');

    screen.getByLabelText('Good response').click();

    await waitFor(() =>
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.observability_submit_score',
        params: { trace_id: 'thread-7:req-42', name: 'user-feedback', value: 1 },
      })
    );
  });

  it('sends 0 for a thumbs down rather than dropping the rating', async () => {
    renderFeedback();

    screen.getByLabelText('Bad response').click();

    await waitFor(() =>
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({ params: expect.objectContaining({ value: 0 }) })
      )
    );
  });

  it('marks the chosen thumb as pressed once the score is recorded', async () => {
    renderFeedback();
    const good = screen.getByLabelText('Good response');
    expect(good).toHaveAttribute('aria-pressed', 'false');

    good.click();

    await waitFor(() => expect(good).toHaveAttribute('aria-pressed', 'true'));
  });

  it('does not resubmit a rating that is already recorded', async () => {
    renderFeedback();
    const good = screen.getByLabelText('Good response');

    good.click();
    await waitFor(() => expect(good).toHaveAttribute('aria-pressed', 'true'));
    good.click();

    expect(mockCallCoreRpc).toHaveBeenCalledTimes(1);
  });

  it('leaves the thumb unpressed when the core reports the score was not recorded', async () => {
    // `ok: false` is the core saying the push was refused (no session, Langfuse
    // rejected the trace, ...). Showing a confirmed rating here would tell the
    // user their feedback landed when it never left the machine.
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    mockCallCoreRpc.mockResolvedValue({ ok: false, error: 'no backend session token' });
    renderFeedback();
    const good = screen.getByLabelText('Good response');

    good.click();

    await waitFor(() => expect(mockCallCoreRpc).toHaveBeenCalled());
    await waitFor(() => expect(good).not.toBeDisabled());
    expect(good).toHaveAttribute('aria-pressed', 'false');
    expect(warn).toHaveBeenCalledWith(
      '[feedback] score was not recorded',
      'no backend session token'
    );
    warn.mockRestore();
  });

  it('recovers when the RPC itself rejects', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    mockCallCoreRpc.mockRejectedValue(new Error('transport down'));
    renderFeedback();
    const good = screen.getByLabelText('Good response');

    good.click();

    await waitFor(() => expect(good).not.toBeDisabled());
    expect(good).toHaveAttribute('aria-pressed', 'false');
    expect(warn).toHaveBeenCalledWith(
      '[feedback] failed to submit response score',
      expect.any(Error)
    );
    warn.mockRestore();
  });
});

describe('MessageFeedbackRail', () => {
  beforeEach(() => vi.clearAllMocks());

  it('renders nothing for a turn that exported no trace', () => {
    // An untraced turn has no trace for a score to attach to, so offering the
    // control would guarantee a dangling score rather than collect feedback.
    const { container } = render(
      <Provider store={store}>
        <MessageFeedbackRail traceId={undefined} />
      </Provider>
    );
    expect(container).toBeEmptyDOMElement();
  });

  it('renders nothing for a non-string or empty trace id', () => {
    for (const value of [42, {}, '', null]) {
      const { container } = render(
        <Provider store={store}>
          <MessageFeedbackRail traceId={value} />
        </Provider>
      );
      expect(container).toBeEmptyDOMElement();
    }
  });

  it('renders the thumbs when the message carries a trace id', () => {
    render(
      <Provider store={store}>
        <MessageFeedbackRail traceId="thread-7:req-42" />
      </Provider>
    );
    expect(screen.getByLabelText('Good response')).toBeInTheDocument();
    expect(screen.getByLabelText('Bad response')).toBeInTheDocument();
  });
});
