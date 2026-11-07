---
status: no-disponible
---

# Flujos de trabajo durables

Corvis no tiene un motor de flujos de trabajo durables (procesos de varios pasos que guardan su estado y se reanudan tras un reinicio o un fallo).

## Alternativa

Para procesos cortos, encadena los pasos en un servicio y guarda el progreso en la base de datos con [corvis-orm](/data/corvis-orm). Para reanudar pasos pendientes, revisa ese estado periódicamente con una tarea `#[cron]` (ver [Tareas programadas](/application/task-scheduling)). Para flujos largos o con compensaciones, usa un motor de workflows externo y llámalo desde un provider.
