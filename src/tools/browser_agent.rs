use crate::agent::Agent;
use crate::agent::prompts::{AgentMessagePrompt, SystemPrompt};
use crate::manager::BrowserManager;
use crate::utils::AgentState;
use kodegen_mcp_schema::browser::{BrowserAgentArgs, BrowserAgentPromptArgs, BROWSER_AGENT, BROWSER_NAVIGATE};
use kodegen_mcp_tool::{Tool, ToolExecutionContext, error::McpError};
use rmcp::model::{Content, PromptArgument, PromptMessage, PromptMessageContent, PromptMessageRole};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct BrowserAgentTool {
    _browser_manager: Arc<BrowserManager>,
    server_url: String,
}

impl BrowserAgentTool {
    pub fn new(browser_manager: Arc<BrowserManager>, server_url: String) -> Self {
        Self {
            _browser_manager: browser_manager,
            server_url,
        }
    }
}

impl Tool for BrowserAgentTool {
    type Args = BrowserAgentArgs;
    type PromptArgs = BrowserAgentPromptArgs;

    fn name() -> &'static str {
        BROWSER_AGENT
    }

    fn description() -> &'static str {
        "Autonomous browser agent that executes multi-step tasks using AI reasoning.\\n\\n\
         The agent can navigate websites, interact with forms, extract information,\\n\
         and complete complex workflows across multiple pages.\\n\\n\
         Example: browser_agent({\\\"task\\\": \\\"Find latest Rust version and save to file\\\", \\\"start_url\\\": \\\"https://rust-lang.org\\\", \\\"max_steps\\\": 8})"
    }

    fn read_only() -> bool {
        false // Agent modifies browser state
    }

    fn open_world() -> bool {
        true // Agent can navigate to any URL
    }

    async fn execute(&self, args: Self::Args, _ctx: ToolExecutionContext) -> Result<Vec<Content>, McpError> {
        // Create loopback MCP client (connects to same server)
        // By the time this tool executes, the server is fully running
        let (mcp_client, _connection) = kodegen_mcp_client::create_streamable_client(
            &self.server_url,
            Default::default(), // Empty header map
        )
        .await
        .map_err(|e| {
            McpError::Other(anyhow::anyhow!(
                "Failed to create loopback client to {}: {}. \
                 Ensure HTTP server is running and accessible.",
                self.server_url,
                e
            ))
        })?;

        // Navigate to start URL if provided (BEFORE creating agent)
        if let Some(url) = &args.start_url {
            mcp_client
                .call_tool(
                    BROWSER_NAVIGATE,
                    json!({
                        "url": url,
                        "timeout_ms": 30000
                    }),
                )
                .await
                .map_err(|e| {
                    McpError::Other(anyhow::anyhow!("Failed to navigate to start URL: {}", e))
                })?;
        }

        // Create agent with all required parameters
        let prompts = crate::agent::PromptConfig {
            system_prompt: SystemPrompt::new(),
            agent_prompt: AgentMessagePrompt::new(),
        };
        let agent_state = Arc::new(Mutex::new(AgentState::new()));

        let config = crate::agent::AgentConfig {
            temperature: args.temperature,
            max_tokens: args.max_tokens,
            vision_timeout_secs: args.vision_timeout_secs,
            llm_timeout_secs: args.llm_timeout_secs,
        };

        let agent = Agent::new(
            &args.task,
            args.additional_info.as_deref().unwrap_or(""),
            Arc::new(mcp_client),
            prompts,
            args.max_actions_per_step as usize,
            agent_state,
            config,
        )
        .map_err(|e| McpError::Other(anyhow::anyhow!("Failed to create agent: {}", e)))?;

        // Execute agent task
        let history = agent
            .run(args.max_steps as usize)
            .await
            .map_err(|e| McpError::Other(anyhow::anyhow!("Agent execution failed: {}", e)))?;

        // Build response with execution summary
        let steps_taken = history.steps.len();
        let is_complete = history.is_complete();
        let final_result = history
            .final_result()
            .unwrap_or_else(|| format!("Agent stopped after {} steps (incomplete)", steps_taken));

        // Extract action summaries from history
        let actions: Vec<Value> = history
            .steps
            .iter()
            .map(|step| {
                json!({
                    "step": step.step,
                    "timestamp": step.timestamp.to_rfc3339(),
                    "actions": step.output.action.iter().map(|a| &a.action).collect::<Vec<_>>(),
                    "summary": step.output.current_state.summary,
                    "complete": step.is_complete,
                })
            })
            .collect();

        let mut contents = Vec::new();

        // Terminal summary
        let status_text = if is_complete { "Complete" } else { "Incomplete" };

        let summary = format!(
            "\x1b[35m󰤖 Browser Agent: {}\x1b[0m\n\
             󰅺 Steps: {}/{} · Status: {}",
            args.task, steps_taken, args.max_steps, status_text
        );
        contents.push(Content::text(summary));

        // JSON metadata
        let metadata = json!({
            "success": is_complete,
            "steps_taken": steps_taken,
            "max_steps": args.max_steps,
            "final_result": final_result,
            "task": args.task,
            "actions": actions,
        });
        let json_str = serde_json::to_string_pretty(&metadata)
            .unwrap_or_else(|_| "{}".to_string());
        contents.push(Content::text(json_str));

        Ok(contents)
    }

    fn prompt_arguments() -> Vec<PromptArgument> {
        vec![PromptArgument {
            name: "focus_area".to_string(),
            title: None,
            description: Some(
                "Optional focus area for examples: 'research' for web research tasks, \
                 'forms' for form-filling automation, 'workflow' for multi-page workflows, \
                 or leave empty for comprehensive overview".to_string(),
            ),
            required: Some(false),
        }]
    }

    async fn prompt(&self, _args: Self::PromptArgs) -> Result<Vec<PromptMessage>, McpError> {
        Ok(vec![
            // Exchange 1: What does browser_agent do?
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text("What is the browser_agent tool and what can it do?"),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(r#"The browser_agent is an autonomous AI-powered browser automation tool that can execute complex multi-step tasks without manual intervention.

Key capabilities:
• Autonomous navigation - Intelligently navigates websites based on task goals
• Vision-based understanding - Uses AI vision to perceive and understand page content
• Multi-step execution - Chains together actions across multiple pages
• Smart interactions - Can click buttons, fill forms, scroll, and extract information
• Self-directed reasoning - Determines next steps based on current page state

Available actions the agent can perform:
1. navigate - Go to URLs
2. click - Click buttons, links, and interactive elements
3. type_text - Enter text into form fields
4. scroll - Scroll pages to reveal content
5. extract_text - Extract information from page elements
6. screenshot - Capture visual state for analysis

Use cases:
• Web research - Gather information across multiple pages
• Form automation - Complete multi-step forms and registrations
• Testing - Verify workflows and user journeys
• Data extraction - Scrape structured information from websites
• Workflow automation - Execute repetitive browser-based tasks"#),
            },
            
            // Exchange 2: When should I use it with examples?
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text("When should I use browser_agent? Give me concrete examples."),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(r#"Use browser_agent when you need multi-step browser automation with intelligent decision-making. Here are three concrete scenarios:

1. WEB RESEARCH TASK
Goal: Find the latest stable Rust version and extract release notes

{
  "task": "Navigate to rust-lang.org, find the latest stable version number, and extract key features from the release notes",
  "start_url": "https://www.rust-lang.org/",
  "max_steps": 8,
  "temperature": 0.7
}

Why these settings:
• start_url: Direct navigation saves a step
• max_steps: 8 allows for navigation → version lookup → release notes → extraction
• temperature: 0.7 provides flexibility for finding information

2. FORM-FILLING TASK
Goal: Complete a contact form with specific information

{
  "task": "Fill out the contact form with name 'Alice Smith', email 'alice@example.com', subject 'Product Inquiry', and message 'Interested in enterprise pricing'",
  "start_url": "https://httpbin.org/forms/post",
  "max_steps": 6,
  "temperature": 0.5,
  "max_actions_per_step": 2
}

Why these settings:
• start_url: Goes directly to the form
• max_steps: 6 is sufficient for filling 4-5 fields and submitting
• temperature: 0.5 for precise, deterministic form filling
• max_actions_per_step: 2 prevents too many actions per iteration

3. MULTI-PAGE WORKFLOW
Goal: Navigate a documentation site to find specific API information

{
  "task": "Find the documentation for async/await syntax in Rust, navigate through the book chapters, and extract code examples",
  "start_url": "https://doc.rust-lang.org/book/",
  "max_steps": 12,
  "temperature": 0.8,
  "vision_timeout_secs": 45,
  "additional_info": "Focus on chapter about asynchronous programming"
}

Why these settings:
• max_steps: 12 allows for multiple page transitions and content extraction
• temperature: 0.8 for flexible navigation through documentation structure
• vision_timeout_secs: 45 for potentially slow-loading documentation pages
• additional_info: Provides context to guide the agent's decisions

Best practices for max_steps:
• Simple tasks (forms, single-page extraction): 5-8 steps
• Medium complexity (2-3 page navigation): 8-12 steps
• Complex workflows (extensive navigation/research): 12-20 steps"#),
            },
            
            // Exchange 3: What parameters should I configure?
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text("How do I configure browser_agent for my task? What do the parameters do?"),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(r#"Configure browser_agent by understanding how each parameter affects execution:

REQUIRED PARAMETERS:

task (string)
• What it does: Describes what the agent should accomplish
• Effect: This is the agent's goal - be specific and clear
• Best practice: Use action-oriented language
• Examples:
  - Good: "Find the latest product pricing and save to a file"
  - Bad: "Look at the website"

OPTIONAL PARAMETERS:

start_url (string, optional)
• What it does: Initial URL to navigate to before starting the task
• Effect: Saves the agent from having to find the starting point
• When to use: When you know the exact starting page
• Default: Agent starts at current page or navigates based on task
• Example: "https://www.rust-lang.org/"

max_steps (integer, default: 8)
• What it does: Maximum number of agent iterations before stopping
• Effect: Limits how long the agent can run
• When to increase: Complex multi-page workflows, extensive navigation
• When to decrease: Simple single-page tasks to prevent over-execution
• Typical ranges:
  - Simple tasks: 5-8
  - Medium tasks: 8-12
  - Complex tasks: 12-20

temperature (float, optional, range: 0.0-2.0, default: varies)
• What it does: Controls AI reasoning creativity and flexibility
• Effect on behavior:
  - Low (0.3-0.5): Deterministic, precise, follows patterns closely
  - Medium (0.6-0.8): Balanced, adaptable to variations
  - High (0.9-1.2): Creative, explores alternatives
• When to use low: Form filling, precise data extraction
• When to use high: Exploratory research, flexible navigation
• Example: 0.5 for filling a form, 0.8 for research

max_tokens (integer, optional, default: 2048)
• What it does: Maximum length of agent's reasoning response per step
• Effect: Limits how much the agent can "think" and plan
• When to increase: Complex reasoning, long action sequences
• When to decrease: Simple tasks to improve speed
• Typical range: 1024-4096

additional_info (string, optional)
• What it does: Provides extra context or constraints to the agent
• Effect: Guides decision-making and priority
• Use cases:
  - Specific preferences: "Prefer official documentation over tutorials"
  - Constraints: "Only extract data from tables, ignore prose"
  - Context: "This is for a security audit, be thorough"

max_actions_per_step (integer, default: 3)
• What it does: Maximum number of browser actions per iteration
• Effect: Controls how much happens in each step
• When to increase (4-5): Complex forms with many fields
• When to decrease (1-2): Careful step-by-step execution
• Default of 3 works well for most cases

vision_timeout_secs (integer, default: 30)
• What it does: Timeout for capturing page screenshots
• Effect: Prevents hanging on slow-loading pages
• When to increase: Slow websites, heavy JavaScript pages
• Typical range: 30-60 seconds

llm_timeout_secs (integer, default: 120)
• What it does: Timeout for AI reasoning calls
• Effect: Prevents hanging on complex reasoning
• When to increase: Very complex tasks requiring deep analysis
• Typical range: 60-180 seconds

PARAMETER COMBINATION PATTERNS:

Precise form filling:
{
  "temperature": 0.5,
  "max_steps": 6,
  "max_actions_per_step": 2
}

Flexible research:
{
  "temperature": 0.8,
  "max_steps": 12,
  "vision_timeout_secs": 45
}

Fast simple task:
{
  "max_steps": 5,
  "max_tokens": 1024,
  "temperature": 0.5
}"#),
            },
            
            // Exchange 4: Best practices and troubleshooting
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text("What are best practices for using browser_agent? How do I fix common issues?"),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(r#"BEST PRACTICES:

1. Write Clear Task Descriptions
• Be specific about the goal and expected outcome
• Good: "Navigate to pricing page, extract all plan prices, and save to a file"
• Bad: "Get pricing info"
• Include success criteria when ambiguous

2. Set Appropriate start_url
• Always provide start_url when you know the starting point
• Saves agent steps and reduces errors
• Ensures consistent starting state

3. Tune Temperature for Task Type
• Deterministic tasks (forms, precise extraction): 0.4-0.6
• Balanced tasks (navigation + extraction): 0.6-0.8
• Exploratory tasks (research, discovery): 0.8-1.0
• Start conservative, increase if agent is too rigid

4. Choose max_steps Based on Complexity
• Count expected steps mentally: navigate (1) + click (1) + extract (1) + save (1) = 4 steps
• Add buffer: 4 × 1.5 = 6 steps minimum
• Better to overshoot than have agent stop prematurely
• Monitor actual steps_taken in results to tune

5. Use additional_info for Context
• Provide constraints: "Only use official sources"
• Give preferences: "Prefer JSON output format"
• Add context: "This data is for financial analysis"
• Clarify ambiguity: "By 'latest', I mean most recent stable release"

6. Monitor Execution Results
• Check steps_taken vs max_steps to see if agent completed
• Review actions array to understand agent's decision-making
• Look at final_result for success/failure indication
• Use is_complete flag to detect premature stops

COMMON ISSUES AND SOLUTIONS:

Issue: "Vision timeout exceeded"
• Symptom: Agent fails with timeout during screenshot capture
• Cause: Page takes too long to load or render
• Solution: Increase vision_timeout_secs to 60 or 90
• Example: { "vision_timeout_secs": 60 }

Issue: "LLM timeout exceeded"
• Symptom: Agent fails during reasoning/decision step
• Cause: Complex page requires extensive analysis
• Solution: Increase llm_timeout_secs to 180 or 240
• Example: { "llm_timeout_secs": 180 }

Issue: "Agent stops early without completing task"
• Symptom: Agent uses all max_steps but task incomplete
• Causes:
  1. max_steps too low for task complexity
  2. Agent getting stuck in loops
  3. Task description unclear
• Solutions:
  - Increase max_steps by 50%: { "max_steps": 12 } → { "max_steps": 18 }
  - Simplify task or break into subtasks
  - Add more context in additional_info
  - Check actions array to see where agent struggled

Issue: "Agent navigates to blank or error pages"
• Symptom: Screenshots show blank pages or 404 errors
• Causes:
  1. start_url not set, agent guessing URLs
  2. Site requires specific navigation path
  3. Page requires authentication
• Solutions:
  - Always set start_url explicitly
  - Add navigation hints in additional_info
  - Pre-navigate to authenticated page before calling agent
  - Example: { "start_url": "https://example.com/dashboard", "additional_info": "Already authenticated" }

Issue: "Form interactions fail or miss fields"
• Symptom: Agent doesn't fill all form fields or clicks wrong elements
• Causes:
  1. Temperature too low, agent too conservative
  2. max_actions_per_step too restrictive
  3. Complex dynamic forms
• Solutions:
  - Increase temperature to 0.6-0.7 for flexibility
  - Increase max_actions_per_step to 4-5
  - Increase vision_timeout_secs for dynamic content
  - Example: { "temperature": 0.7, "max_actions_per_step": 4, "vision_timeout_secs": 45 }

Issue: "Agent performs too many unnecessary actions"
• Symptom: Agent clicks/scrolls excessively without progress
• Causes:
  1. Temperature too high, agent too exploratory
  2. Task description too vague
  3. max_steps too high allowing wandering
• Solutions:
  - Decrease temperature to 0.4-0.5
  - Make task description more specific
  - Add constraints in additional_info
  - Example: { "temperature": 0.5, "max_steps": 8, "additional_info": "Only extract data from the main table, ignore sidebar" }

Issue: "Cannot extract specific information"
• Symptom: Agent completes but doesn't return expected data
• Causes:
  1. Task description doesn't specify extraction
  2. Information not visible to agent
  3. Agent misidentifies content
• Solutions:
  - Explicitly mention extraction in task: "...and extract the price"
  - Describe location: "...from the pricing table"
  - Increase temperature for better content recognition
  - Add hints in additional_info about content structure

DEBUGGING WORKFLOW:

1. Start with conservative settings and clear task
2. Run and check is_complete flag
3. If incomplete, examine steps_taken vs max_steps
4. Review actions array to see where agent struggled
5. Adjust relevant parameter (timeouts, steps, temperature)
6. Iterate until reliable

Example debugging progression:
First run: { "task": "Get pricing", "max_steps": 8 }
→ Agent stops early, only 3 steps taken
Second run: { "task": "Navigate to pricing page and extract all plan prices", "max_steps": 8, "start_url": "https://example.com" }
→ Works but timeout on complex page
Third run: { "task": "Navigate to pricing page and extract all plan prices", "max_steps": 8, "start_url": "https://example.com", "vision_timeout_secs": 60 }
→ Success!"#),
            },
        ])
    }
}
