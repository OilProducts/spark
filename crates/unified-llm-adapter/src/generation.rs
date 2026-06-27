use crate::client::Client;
use crate::errors::AdapterError;
use crate::request::{Request, Response};
use crate::retry::{retry, retry_with_hooks, RetryPolicy};

#[derive(Debug, Clone, PartialEq)]
pub struct GenerateStep {
    pub request: Request,
    pub response: Response,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GenerateResult {
    pub text: String,
    pub response: Response,
    pub steps: Vec<GenerateStep>,
}

pub fn generate(client: &Client, request: Request) -> Result<GenerateResult, AdapterError> {
    generate_with_policy(client, request, &RetryPolicy::default())
}

pub fn generate_with_policy(
    client: &Client,
    request: Request,
    policy: &RetryPolicy,
) -> Result<GenerateResult, AdapterError> {
    generate_steps_with_policy(client, request, policy, |_steps| Ok(None))
}

pub fn generate_with_policy_and_hooks<R, S>(
    client: &Client,
    request: Request,
    policy: &RetryPolicy,
    random_multiplier: R,
    sleeper: S,
) -> Result<GenerateResult, AdapterError>
where
    R: FnMut() -> f64,
    S: FnMut(f64),
{
    generate_steps_with_policy_and_hooks(
        client,
        request,
        policy,
        |_steps| Ok(None),
        random_multiplier,
        sleeper,
    )
}

pub fn generate_steps_with_policy<N>(
    client: &Client,
    initial_request: Request,
    policy: &RetryPolicy,
    mut next_request: N,
) -> Result<GenerateResult, AdapterError>
where
    N: FnMut(&[GenerateStep]) -> Result<Option<Request>, AdapterError>,
{
    let mut current_request = initial_request;
    let mut steps = Vec::new();

    loop {
        let request_for_call = current_request.clone();
        let response = retry(policy, || client.complete(request_for_call.clone()))?;
        steps.push(GenerateStep {
            request: current_request,
            response,
        });

        let Some(next) = next_request(&steps)? else {
            return finish_generation(steps);
        };
        current_request = next;
    }
}

pub fn generate_steps_with_policy_and_hooks<N, R, S>(
    client: &Client,
    initial_request: Request,
    policy: &RetryPolicy,
    mut next_request: N,
    mut random_multiplier: R,
    mut sleeper: S,
) -> Result<GenerateResult, AdapterError>
where
    N: FnMut(&[GenerateStep]) -> Result<Option<Request>, AdapterError>,
    R: FnMut() -> f64,
    S: FnMut(f64),
{
    let mut current_request = initial_request;
    let mut steps = Vec::new();

    loop {
        let request_for_call = current_request.clone();
        let response = retry_with_hooks(
            policy,
            || client.complete(request_for_call.clone()),
            &mut random_multiplier,
            &mut sleeper,
        )?;
        steps.push(GenerateStep {
            request: current_request,
            response,
        });

        let Some(next) = next_request(&steps)? else {
            return finish_generation(steps);
        };
        current_request = next;
    }
}

fn finish_generation(steps: Vec<GenerateStep>) -> Result<GenerateResult, AdapterError> {
    let response = steps
        .last()
        .map(|step| step.response.clone())
        .expect("generation records at least one step before finishing");
    Ok(GenerateResult {
        text: response.text(),
        response,
        steps,
    })
}
