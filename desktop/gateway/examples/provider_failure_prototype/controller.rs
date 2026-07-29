#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_posts: u8,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
    pub retry_after_cap_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_posts: 3,
            base_delay_ms: 500,
            max_delay_ms: 2_000,
            retry_after_cap_ms: 60_000,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AttemptState {
    pub posts_started: u8,
    pub repairs_used: u8,
    pub response_started: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteContext {
    pub provider: String,
    pub route: String,
    pub correlation_id: String,
    pub policy: RetryPolicy,
}

#[derive(Clone, Debug)]
pub struct AttemptController {
    context: RouteContext,
    state: AttemptState,
}

impl AttemptController {
    pub fn new(context: RouteContext) -> Self {
        Self {
            context,
            state: AttemptState::default(),
        }
    }

    pub fn context(&self) -> &RouteContext {
        &self.context
    }

    pub fn state(&self) -> &AttemptState {
        &self.state
    }

    pub fn begin_post(&mut self) -> bool {
        if self.state.response_started
            || self.state.posts_started >= self.context.policy.max_posts
        {
            return false;
        }
        self.state.posts_started += 1;
        true
    }

    pub fn mark_response_started(&mut self) -> bool {
        if self.state.posts_started == 0 {
            return false;
        }
        self.state.response_started = true;
        true
    }

    pub fn reset(&mut self) {
        self.state = AttemptState::default();
    }
}
