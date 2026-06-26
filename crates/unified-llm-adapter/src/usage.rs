use serde::{Deserialize, Serialize};

use crate::catalog::ModelInfo;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub reasoning_tokens: Option<u64>,
    #[serde(default)]
    pub cache_read_tokens: Option<u64>,
    #[serde(default)]
    pub cache_write_tokens: Option<u64>,
    #[serde(default)]
    pub raw: Option<serde_json::Value>,
}

impl Usage {
    pub fn normalized(mut self) -> Self {
        if self.total_tokens == 0 {
            self.total_tokens = self.input_tokens + self.output_tokens;
        }
        self
    }

    pub fn cost_for_model(&self, model: &ModelInfo) -> Option<CostEstimate> {
        let input_rate = model.input_cost_per_million?;
        let output_rate = model.output_cost_per_million?;
        let input_cost = (self.input_tokens as f64 / 1_000_000.0) * input_rate;
        let output_cost = (self.output_tokens as f64 / 1_000_000.0) * output_rate;
        Some(CostEstimate {
            input_cost,
            output_cost,
            total_cost: input_cost + output_cost,
        })
    }
}

impl std::ops::Add for Usage {
    type Output = Usage;

    fn add(self, rhs: Self) -> Self::Output {
        Usage {
            input_tokens: self.input_tokens + rhs.input_tokens,
            output_tokens: self.output_tokens + rhs.output_tokens,
            total_tokens: self.total_tokens + rhs.total_tokens,
            reasoning_tokens: sum_optional_tokens(self.reasoning_tokens, rhs.reasoning_tokens),
            cache_read_tokens: sum_optional_tokens(self.cache_read_tokens, rhs.cache_read_tokens),
            cache_write_tokens: sum_optional_tokens(
                self.cache_write_tokens,
                rhs.cache_write_tokens,
            ),
            raw: self.raw.or(rhs.raw),
        }
        .normalized()
    }
}

fn sum_optional_tokens(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (None, None) => None,
        (left, right) => Some(left.unwrap_or(0) + right.unwrap_or(0)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CostEstimate {
    pub input_cost: f64,
    pub output_cost: f64,
    pub total_cost: f64,
}
