/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Thread、Turn、Item 的单向状态迁移规则。

use super::{ItemStatus, ThreadRepositoryError, ThreadStatus, TurnStatus};

pub(super) fn ensure_thread_transition(current: ThreadStatus, next: ThreadStatus) -> Result<(), ThreadRepositoryError> {
    if current == next
        || matches!(
            (current, next),
            (ThreadStatus::Active, ThreadStatus::Archived) | (ThreadStatus::Archived, ThreadStatus::Active)
        )
    {
        Ok(())
    } else {
        Err(ThreadRepositoryError::InvalidTransition(format!(
            "thread cannot transition from {} to {}",
            current.as_str(),
            next.as_str()
        )))
    }
}

pub(super) fn ensure_turn_transition(current: TurnStatus, next: TurnStatus) -> Result<(), ThreadRepositoryError> {
    let allowed = current == next
        || matches!(
            (current, next),
            (TurnStatus::Created, TurnStatus::Running)
                | (
                    TurnStatus::Created,
                    TurnStatus::Cancelled | TurnStatus::Failed | TurnStatus::Interrupted | TurnStatus::BudgetExhausted
                )
                | (TurnStatus::Running, TurnStatus::Verifying)
                | (TurnStatus::Verifying, TurnStatus::Running)
                | (
                    TurnStatus::Running,
                    TurnStatus::WaitingFrontendTool | TurnStatus::WaitingUserInput | TurnStatus::WaitingUserConfirmation
                )
                | (
                    TurnStatus::Running | TurnStatus::Verifying,
                    TurnStatus::Completed | TurnStatus::Cancelled | TurnStatus::Failed | TurnStatus::Interrupted | TurnStatus::BudgetExhausted
                )
                | (
                    TurnStatus::WaitingFrontendTool | TurnStatus::WaitingUserInput | TurnStatus::WaitingUserConfirmation,
                    TurnStatus::Running | TurnStatus::Cancelled | TurnStatus::Failed | TurnStatus::Interrupted | TurnStatus::BudgetExhausted
                )
        );
    if allowed {
        Ok(())
    } else {
        Err(ThreadRepositoryError::InvalidTransition(format!(
            "turn cannot transition from {} to {}",
            current.as_str(),
            next.as_str()
        )))
    }
}

pub(super) fn ensure_item_transition(current: ItemStatus, next: ItemStatus) -> Result<(), ThreadRepositoryError> {
    let allowed = current == next
        || matches!(
            (current, next),
            (ItemStatus::Created, ItemStatus::InProgress | ItemStatus::Waiting | ItemStatus::Completed)
                | (
                    ItemStatus::InProgress,
                    ItemStatus::Waiting | ItemStatus::Completed | ItemStatus::Failed | ItemStatus::Cancelled
                )
                | (ItemStatus::Waiting, ItemStatus::Completed | ItemStatus::Failed | ItemStatus::Declined | ItemStatus::Cancelled)
        );
    if allowed {
        Ok(())
    } else {
        Err(ThreadRepositoryError::InvalidTransition(format!(
            "item cannot transition from {} to {}",
            current.as_str(),
            next.as_str()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_turn_and_item_states_cannot_reopen() {
        assert!(ensure_turn_transition(TurnStatus::Completed, TurnStatus::Running).is_err());
        assert!(ensure_item_transition(ItemStatus::Completed, ItemStatus::InProgress).is_err());
        assert!(ensure_turn_transition(TurnStatus::WaitingUserInput, TurnStatus::Running).is_ok());
    }
}
