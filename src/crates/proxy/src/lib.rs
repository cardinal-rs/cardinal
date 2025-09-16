mod utils;

use cardinal_base::context::CardinalContext;
use std::sync::Arc;

pub struct CardinalProxy {
    context: Arc<CardinalContext>,
}

impl CardinalProxy {
    pub fn new(context: Arc<CardinalContext>) -> Self {
        Self { context }
    }
}
