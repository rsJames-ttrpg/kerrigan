use sea_query::Iden;

#[derive(Iden)]
pub enum Memories {
    Table,
    Id,
    Content,
    EmbeddingModel,
    Source,
    Tags,
    ExpiresAt,
    CreatedAt,
    UpdatedAt,
}

#[derive(Iden)]
pub enum MemoryLinks {
    Table,
    MemoryId,
    LinkedId,
    LinkedType,
    RelationType,
}

#[derive(Iden)]
pub enum JobDefinitions {
    Table,
    Id,
    Name,
    Description,
    Config,
    CreatedAt,
    UpdatedAt,
}

#[derive(Iden)]
pub enum JobRuns {
    Table,
    Id,
    DefinitionId,
    ParentId,
    Status,
    TriggeredBy,
    Result,
    Error,
    StartedAt,
    CompletedAt,
}

#[derive(Iden)]
pub enum Tasks {
    Table,
    Id,
    RunId,
    Subject,
    Status,
    AssignedTo,
    Output,
    CreatedAt,
    UpdatedAt,
}

#[derive(Iden)]
pub enum Decisions {
    Table,
    Id,
    Agent,
    Context,
    Decision,
    Reasoning,
    Tags,
    RunId,
    CreatedAt,
}

#[derive(Iden)]
pub enum Artifacts {
    Table,
    Id,
    Name,
    ContentType,
    Size,
    RunId,
    CreatedAt,
}
