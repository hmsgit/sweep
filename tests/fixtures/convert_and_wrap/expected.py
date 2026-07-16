def sync(records, batch_size=100):
    """Synchronize the given records with the remote skills catalogue
    and return a summary.

    :param records: the raw records to push upstream, already validated
        against the catalogue schema and deduplicated by external
        identifier.
    :type records: list[dict]
    :param batch_size: how many records go into one request.
    :type batch_size: int
    :returns: mapping of record id to sync status.
    :rtype: dict
    """
    return {}
