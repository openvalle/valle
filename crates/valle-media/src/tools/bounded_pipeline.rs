//! Small synchronous queue with an explicit logical capacity and observable occupancy.

use std::collections::VecDeque;

use super::{PipelineUsage, ToolError, ToolErrorCode};

/// A queue whose logical capacity comes from [`super::ResourcePolicy`].
///
/// `VecDeque::new` deliberately avoids reserving the entire policy value up front. The queue
/// allocates only for items actually pushed while still rejecting any occupancy above the limit.
#[derive(Debug)]
pub(super) struct BoundedPipeline<T> {
    items: VecDeque<T>,
    capacity: usize,
    high_watermark: usize,
}

impl<T> BoundedPipeline<T> {
    pub(super) fn new(capacity: usize) -> Result<Self, ToolError> {
        if capacity == 0 {
            return Err(ToolError::new(
                ToolErrorCode::InvalidInput,
                "bounded pipeline capacity must be greater than zero",
            ));
        }
        Ok(Self {
            items: VecDeque::new(),
            capacity,
            high_watermark: 0,
        })
    }

    #[cfg(any(feature = "tool-upscale", test))]
    pub(super) const fn capacity(&self) -> usize {
        self.capacity
    }

    #[cfg(any(feature = "tool-upscale", test))]
    pub(super) fn len(&self) -> usize {
        self.items.len()
    }

    pub(super) fn push_back(&mut self, item: T) -> Result<(), ToolError> {
        if self.items.len() == self.capacity {
            return Err(ToolError::new(
                ToolErrorCode::ResourceBusy,
                format!(
                    "bounded pipeline occupancy would exceed its capacity of {}",
                    self.capacity
                ),
            ));
        }
        self.items.push_back(item);
        self.high_watermark = self.high_watermark.max(self.items.len());
        Ok(())
    }

    pub(super) fn pop_front(&mut self) -> Option<T> {
        self.items.pop_front()
    }

    #[cfg(any(feature = "tool-upscale", test))]
    pub(super) fn front(&self) -> Option<&T> {
        self.items.front()
    }

    #[cfg(any(feature = "tool-upscale", test))]
    pub(super) fn get(&self, index: usize) -> Option<&T> {
        self.items.get(index)
    }

    pub(super) const fn usage(&self) -> PipelineUsage {
        PipelineUsage {
            capacity: self.capacity,
            high_watermark: self.high_watermark,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_rejects_over_capacity_and_reports_its_high_watermark() {
        let mut queue = BoundedPipeline::new(2).unwrap();
        assert_eq!(queue.capacity(), 2);
        assert_eq!(queue.len(), 0);
        queue.push_back(1).unwrap();
        queue.push_back(2).unwrap();
        assert_eq!(queue.front(), Some(&1));
        assert_eq!(queue.get(1), Some(&2));
        assert_eq!(
            queue.push_back(3).unwrap_err().code,
            ToolErrorCode::ResourceBusy
        );
        assert_eq!(
            queue.usage(),
            PipelineUsage {
                capacity: 2,
                high_watermark: 2
            }
        );
        assert_eq!(queue.pop_front(), Some(1));
        queue.push_back(3).unwrap();
        assert_eq!(
            queue.usage(),
            PipelineUsage {
                capacity: 2,
                high_watermark: 2
            }
        );
    }

    #[test]
    fn zero_capacity_is_rejected_without_allocating_a_queue() {
        assert_eq!(
            BoundedPipeline::<()>::new(0).unwrap_err().code,
            ToolErrorCode::InvalidInput
        );
    }
}
