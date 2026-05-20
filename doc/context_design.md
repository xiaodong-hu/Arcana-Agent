# Harness Design --- Agent to LLM Interactive

## 1. Context Management

### 1.1 Interfact Exposed to LLMs
To reduce the waste of context for LLM to "guess" what is on hand, what is doable, and to avoid many trials block by the rust-managed authority program, the agent would always start dialogue with telling of what kind of registered and authoritzed tools/network commands etc are doable for current dialogue. Also LLM should be able to send a query of list of what is loaded/registered/authorized.

These hard-coded header should include:
- authorized system tools/commands
- authorized network tools (such as `wget` or `curl`)
- authorized user-skill defined tools/commands

### 1.2 Long-term Memory Exposed to LLMs
All long-term system memory should be append to the context to the head of session (only once). It includes `SOUL.md`, `USER.md` etc.
An explicit API interface to query and write the long-term knowledge/error vector database should also be explicitly exposed to LLMs.


### 1.2 Contex Window Exposed to LLMs
To reduce the waste usage of token, the project memory (markdown file), and interfac to query and write session vector memory should also be explicitly exposed to LLMs.

Another important thing is the thinking chain, I would like to further maintain the thinking chain as some `thinking_chain` vector memory database for each project. LLM can decide which to write and query for that (this needs change to the memory architecture)


## Mode Design (toggled with `/mode`)
### Ask Mode
The authority program should add this mode so that, NO mutation can be made by LLMs, it can only read projects, extra files, and use web tools. Project and session memory should be loaded as well.

### Agent Mode
Normal Mode


### Plan Mode
TODO!